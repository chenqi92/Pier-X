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
    let wrapped = format!("sudo -S -p '' bash -c '{escaped}'");
    let stdin = format!("{password}\n");
    (wrapped, stdin)
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
        // generic EACCES / EPERM message
        || lower.contains("eacces")
        || lower.contains("eperm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_simple_command() {
        let (cmd, stdin) = wrap_command("docker ps", "hunter2");
        assert_eq!(cmd, "sudo -S -p '' bash -c 'docker ps'");
        assert_eq!(stdin, "hunter2\n");
    }

    #[test]
    fn wraps_command_with_single_quotes() {
        let (cmd, _) = wrap_command("echo 'hi'", "pw");
        // ' becomes '\''
        assert_eq!(cmd, r"sudo -S -p '' bash -c 'echo '\''hi'\'''");
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
        assert!(is_permission_denied(
            "Interactive authentication required."
        ));
    }

    #[test]
    fn does_not_flag_unrelated_errors() {
        assert!(!is_permission_denied("No such file or directory"));
        assert!(!is_permission_denied("Container not found"));
    }
}
