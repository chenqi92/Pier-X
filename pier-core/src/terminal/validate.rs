//! Command-name validation for smart-mode typo highlighting.
//!
//! Smart Mode highlights commands the user is typing. When the
//! command name doesn't resolve to anything the shell could execute
//! — neither a builtin, nor a binary on `$PATH` — the overlay paints
//! it red so the user notices the typo before pressing Enter.
//!
//! This module is the resolver. It is deliberately conservative:
//!
//! * **Builtins** are matched against a static union of POSIX +
//!   bash + zsh built-in names. False negatives (a shell-specific
//!   builtin we forgot) just look like plain text — harmless. False
//!   positives (a name in our table that the user's shell doesn't
//!   actually expose) would mark a typo as valid; the table sticks
//!   to widely-supported names to keep that to near zero.
//! * **Binaries** are looked up by walking `$PATH` and checking the
//!   executable bit (Unix) or the `.exe` / `.bat` / `.cmd` / `.com`
//!   extensions (Windows). The first hit wins, matching how the
//!   shell itself resolves names.
//! * **Aliases / functions** declared at runtime in the user's
//!   `.bashrc` / `.zshrc` are out of scope for M3 — we'd need to
//!   query the live shell, which is M4+ work. Until then a custom
//!   `gs` alias for `git status` will be flagged red. The plan
//!   acknowledges this: the user can disable smart mode if their
//!   alias setup is heavy.
//!
//! The function does not take a session id because `$PATH` is a
//! per-process value and Pier-X inherits the user's login env. If
//! the user mutates `PATH` inside a running shell (e.g. `export
//! PATH=...`) the discrepancy is small in practice and improving it
//! requires reading `/proc/<pid>/environ`, which is M4+ scope.

use std::path::PathBuf;

/// What kind of command `name` resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandKind {
    /// Shell built-in (cd, echo, pwd, …). No filesystem path.
    Builtin,
    /// Found in `$PATH`. Carries the absolute path of the first
    /// hit so the UI can show it as a tooltip / hover.
    Binary(PathBuf),
    /// Not a builtin and not in `$PATH`. Highlighted as a typo.
    Missing,
}

/// Union of POSIX, bash, and zsh built-ins. Names that exist in
/// only one of those shells but never the others are still
/// included — false positives just mean the highlight reads as
/// "valid" instead of "typo", and the user never sees a wrong-
/// looking shell error.
pub const SHELL_BUILTINS: &[&str] = &[
    // POSIX special builtins
    ":",
    ".",
    "break",
    "continue",
    "eval",
    "exec",
    "exit",
    "export",
    "readonly",
    "return",
    "set",
    "shift",
    "trap",
    "unset",
    // POSIX regular builtins
    "alias",
    "bg",
    "cd",
    "command",
    "echo",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "jobs",
    "kill",
    "let",
    "newgrp",
    "pwd",
    "read",
    "test",
    "[",
    "]",
    "times",
    "true",
    "type",
    "ulimit",
    "umask",
    "unalias",
    "wait",
    "history",
    // bash extras (also commonly available in zsh)
    "bind",
    "builtin",
    "caller",
    "compgen",
    "complete",
    "compopt",
    "declare",
    "dirs",
    "disown",
    "enable",
    "help",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "readarray",
    "shopt",
    "source",
    "suspend",
    "typeset",
    // zsh-specific
    "autoload",
    "bindkey",
    "chdir",
    "compdef",
    "limit",
    "noglob",
    "rehash",
    "sched",
    "setopt",
    "stat",
    "ttyctl",
    "unhash",
    "unlimit",
    "unsetopt",
    "vared",
    "whence",
    "where",
    "which",
    "zcompile",
    "zformat",
    "zle",
    "zmodload",
    "zparseopts",
    "zprof",
    "zpty",
    "zregexparse",
    "zsocket",
    "zstat",
    "zstyle",
];

/// Resolve `name` against builtins and `$PATH`.
///
/// Words containing a path separator are intentionally returned as
/// `Missing` here — the lexer should already classify them as
/// `path` tokens and never call this function on them. Returning
/// `Missing` for path-shaped input is a defensive fallback so a
/// caller mistake doesn't show `/bin/ls` as a valid command twice.
pub fn validate_command(name: &str) -> CommandKind {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return CommandKind::Missing;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return CommandKind::Missing;
    }
    if SHELL_BUILTINS.iter().any(|b| *b == trimmed) {
        return CommandKind::Builtin;
    }

    let separator = if cfg!(windows) { ';' } else { ':' };
    let path_var = match std::env::var("PATH") {
        Ok(p) => p,
        Err(_) => return CommandKind::Missing,
    };

    for dir in path_var.split(separator) {
        if dir.is_empty() {
            continue;
        }
        let candidate = PathBuf::from(dir).join(trimmed);
        if is_executable(&candidate) {
            return CommandKind::Binary(candidate);
        }
        // Windows executable extensions. PATHEXT controls the real
        // list at runtime; we use the canonical four to avoid the
        // additional env var roundtrip — `cmd.exe` defaults to this
        // set anyway.
        #[cfg(windows)]
        {
            for ext in &["exe", "bat", "cmd", "com"] {
                let with_ext = candidate.with_extension(ext);
                if with_ext.is_file() {
                    return CommandKind::Binary(with_ext);
                }
            }
        }
    }

    CommandKind::Missing
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

    #[test]
    fn empty_input_is_missing() {
        assert_eq!(validate_command(""), CommandKind::Missing);
        assert_eq!(validate_command("   "), CommandKind::Missing);
    }

    #[test]
    fn names_with_slashes_are_missing() {
        // The lexer classifies these as `path` tokens; the validator
        // returns Missing as a defensive fallback if it's ever called.
        assert_eq!(validate_command("/bin/ls"), CommandKind::Missing);
        assert_eq!(validate_command("./script.sh"), CommandKind::Missing);
        assert_eq!(validate_command("../tools/x"), CommandKind::Missing);
    }

    #[test]
    fn cd_is_recognised_as_builtin() {
        assert_eq!(validate_command("cd"), CommandKind::Builtin);
        assert_eq!(validate_command("echo"), CommandKind::Builtin);
        assert_eq!(validate_command("export"), CommandKind::Builtin);
    }

    #[test]
    fn typo_resolves_to_missing() {
        // "gti" / "lss" / "exitt" are typos of common commands and
        // not on PATH on any sane system.
        assert_eq!(validate_command("gti"), CommandKind::Missing);
        assert_eq!(
            validate_command("definitely-not-a-real-binary-pierx"),
            CommandKind::Missing,
        );
    }

    #[test]
    fn ls_or_sh_is_a_binary_on_unix() {
        // On Unix systems running this test, /bin/ls is virtually
        // always present. We assert via PATH lookup so the test
        // works whatever the runner's PATH actually contains.
        if cfg!(unix) {
            match validate_command("ls") {
                CommandKind::Binary(p) => {
                    assert!(p.is_absolute(), "binary path should be absolute, got {p:?}");
                    assert!(
                        p.file_name().map(|n| n == "ls").unwrap_or(false),
                        "expected the resolved path to end in `ls`, got {p:?}",
                    );
                }
                other => panic!("expected ls to resolve to Binary, got {other:?}"),
            }
        }
    }

    #[test]
    fn builtins_table_contains_the_canonical_set() {
        // Sanity check that the table keeps the most common
        // builtins. If a future edit accidentally drops these the
        // overlay will start flagging cd / echo / pwd as typos —
        // very visible regression we want to fail loudly on.
        for must_have in &["cd", "echo", "pwd", "exit", "set", "export", "alias"] {
            assert!(
                SHELL_BUILTINS.contains(must_have),
                "{must_have} is missing from SHELL_BUILTINS",
            );
        }
    }
}
