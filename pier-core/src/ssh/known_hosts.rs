//! Host key verification.
//!
//! ## M3c4: real OpenSSH known_hosts support
//!
//! M3a shipped an `AcceptAllLogFingerprint` placeholder that
//! accepted every server key it saw. M3c4 replaces that with a
//! real `OpenSshKnownHosts` variant that reads an OpenSSH-format
//! `known_hosts` file, parses host→key pairs via russh's
//! `check_known_hosts_path` helper, and:
//!
//!  * accepts keys that match a pinned entry,
//!  * rejects keys that conflict with a pinned entry
//!    (russh returns `Error::KeyChanged { line }`, we surface it
//!    as [`super::SshError::HostKeyMismatch`]),
//!  * "trusts on first use" any host that has no pinned entry
//!    yet — we append the key to the known_hosts file via
//!    russh's `learn_known_hosts_path`. This matches the
//!    OpenSSH `StrictHostKeyChecking=accept-new` behaviour,
//!    which is the most ergonomic safe-by-default setting for
//!    an IDE-style SSH client.
//!
//! The default variant also goes through the real verifier
//! now (pointing at `~/.ssh/known_hosts`). `AcceptAllLogFingerprint`
//! stays in the enum for tests and for users who explicitly
//! want to bypass verification.
//!
//! ## What does NOT live here
//!
//! Any shell prompt. A "trust this new host?" interaction would
//! be the safest-by-default UX, but it requires a round-trip
//! through the desktop command/event layer from inside an async
//! russh handler, which is non-trivial. M3c5 or later can add a
//! "paranoid mode" verifier that holds up the handshake on a
//! channel and waits for a user answer; M3c4 ships
//! accept-on-first-use which is the widely-accepted compromise.

use std::path::PathBuf;

use russh::keys::ssh_key::PublicKey;

use crate::paths;

/// How an SSH session decides whether to trust a server's host key.
#[derive(Debug, Clone)]
pub enum HostKeyVerifier {
    /// Parse an OpenSSH-format known_hosts file, match server
    /// keys against it, and append new keys on first connect
    /// (accept-new / trust-on-first-use semantics).
    ///
    /// This is the default constructed by [`HostKeyVerifier::default`]
    /// pointing at `~/.ssh/known_hosts`.
    OpenSshKnownHosts {
        /// Absolute path to the known_hosts file. Created on
        /// first write if it does not yet exist.
        path: PathBuf,
    },

    /// Accept every server key and log the fingerprint. Tests
    /// use this to avoid touching the user's real known_hosts
    /// file; production builds should not.
    AcceptAllLogFingerprint,
}

impl HostKeyVerifier {
    /// Verify a server-presented key for the given host. Returns
    /// `Ok(true)` to accept, `Ok(false)` to reject, `Err(_)` on
    /// parse / I/O failure reading the known_hosts file.
    ///
    /// On an unknown host (no existing pinned entry) with the
    /// `OpenSshKnownHosts` variant, this ALSO appends the new
    /// key to the file — that's the accept-on-first-use
    /// behaviour. Callers that want stricter behaviour should
    /// wrap this in a higher-level check.
    pub fn verify(&self, host: &str, port: u16, key: &PublicKey) -> Result<bool, VerifyError> {
        match self {
            Self::AcceptAllLogFingerprint => {
                let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256);
                log::info!(
                    "ssh host key for {host}:{port} (AcceptAll, M3a verifier): {fingerprint}",
                );
                Ok(true)
            }
            Self::OpenSshKnownHosts { path } => {
                match russh::keys::known_hosts::check_known_hosts_path(host, port, key, path) {
                    // Existing matching entry → trust.
                    Ok(true) => {
                        log::debug!("host key for {host}:{port} matches known_hosts pin");
                        Ok(true)
                    }
                    // No existing entry → learn it (TOFU).
                    Ok(false) => {
                        // Make sure the directory exists before
                        // learn_known_hosts_path opens the file
                        // for append. A fresh `pier-x` profile
                        // on a new machine won't have ~/.ssh yet.
                        if let Some(parent) = path.parent() {
                            if !parent.exists() {
                                if let Err(e) = std::fs::create_dir_all(parent) {
                                    log::warn!(
                                        "failed to create known_hosts parent {parent:?}: {e}",
                                    );
                                }
                                // Best-effort chmod 700 on the
                                // freshly-created directory —
                                // matches what ssh-keygen does.
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    let _ = std::fs::set_permissions(
                                        parent,
                                        std::fs::Permissions::from_mode(0o700),
                                    );
                                }
                            }
                        }
                        russh::keys::known_hosts::learn_known_hosts_path(host, port, key, path)
                            .map_err(|e| VerifyError::Io(format!("{e}")))?;
                        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256);
                        log::info!("learned new host key for {host}:{port} (TOFU): {fingerprint}",);
                        Ok(true)
                    }
                    // An existing entry that doesn't match →
                    // surface as a structured error so the
                    // higher layers can translate to
                    // SshError::HostKeyMismatch.
                    Err(russh::keys::Error::KeyChanged { line }) => {
                        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256);
                        log::warn!(
                            "host key for {host}:{port} MISMATCH at {path:?} line {line}: {fingerprint}",
                        );
                        Err(VerifyError::Mismatch {
                            host: host.to_string(),
                            fingerprint: format!("{fingerprint}"),
                            line,
                        })
                    }
                    Err(e) => Err(VerifyError::Io(format!("{e}"))),
                }
            }
        }
    }
}

impl Default for HostKeyVerifier {
    fn default() -> Self {
        // Default to the real verifier pointing at the
        // OpenSSH-standard location. If the platform doesn't
        // expose a home directory (which should never happen in
        // practice on Unix / Windows / macOS), fall back to the
        // accept-all variant and log — failing loudly here would
        // keep pier-x from running at all on an unusual setup.
        match default_known_hosts_path() {
            Some(path) => Self::OpenSshKnownHosts { path },
            None => {
                log::warn!("no resolvable home directory; falling back to AcceptAllLogFingerprint",);
                Self::AcceptAllLogFingerprint
            }
        }
    }
}

/// Resolve `~/.ssh/known_hosts` for the current user without
/// pulling in any OpenSSH-specific dependencies.
fn default_known_hosts_path() -> Option<PathBuf> {
    // directories::UserDirs would be cleaner but we already
    // depend on directories for data_dir(); the existing
    // paths module doesn't expose a home-dir helper though,
    // so do the simple env-based lookup here. Works on both
    // Unix ($HOME) and Windows (%USERPROFILE% via the same env
    // var on most installations), and has no fallible parse
    // step that would surface a confusing error.
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".ssh").join("known_hosts"))
}

/// Per-file override: write the known_hosts path into the
/// pier-x data directory instead of `~/.ssh/known_hosts`.
/// Used by the (not-yet-built) "isolated profile" mode that
/// keeps pier-x from touching the user's real SSH state.
#[allow(dead_code)]
pub fn pier_x_known_hosts_path() -> Option<PathBuf> {
    paths::data_dir().map(|d| d.join("known_hosts"))
}

/// Errors the verifier can produce. Separate from
/// [`crate::ssh::SshError`] so the `SshSession::connect`
/// handler can match on `Mismatch` specifically and translate
/// to [`crate::ssh::SshError::HostKeyMismatch`] with the full
/// UI-friendly structure.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// An existing known_hosts entry for this host does not
    /// match the server-presented key. Surface to the user
    /// as a security warning.
    #[error("host key mismatch for {host} at line {line}: {fingerprint}")]
    Mismatch {
        /// Hostname that was being dialed.
        host: String,
        /// SHA-256 fingerprint of the new key the server presented.
        fingerprint: String,
        /// Line number in known_hosts of the conflicting entry.
        line: usize,
    },

    /// I/O or parse error reading the known_hosts file.
    #[error("known_hosts verifier I/O: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fresh_tmp_path(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        temp_dir().join(format!("pier-x-test-khosts-{label}-{pid}-{n}"))
    }

    /// Build a deterministic ed25519 public key for tests from
    /// a single seed byte. The returned `PublicKey` has an
    /// **empty comment** — russh's `learn_known_hosts_path`
    /// writes the key with its comment and `check_known_hosts_path`
    /// reads it back via `parse_public_key_base64` which
    /// discards the comment, so test keys with non-empty
    /// comments would round-trip as "not equal" to themselves
    /// and break the TOFU test path. Empty comments round-trip
    /// identically.
    fn make_test_key(seed: u8) -> PublicKey {
        use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
        use russh::keys::ssh_key::PrivateKey;

        let seed_bytes = [seed; 32];
        let keypair = Ed25519Keypair::from_seed(&seed_bytes);
        let pk = PrivateKey::new(KeypairData::Ed25519(keypair), "")
            .expect("constructing ed25519 PrivateKey from seed must succeed");
        pk.public_key().clone()
    }

    #[test]
    fn accept_all_variant_always_returns_true() {
        let v = HostKeyVerifier::AcceptAllLogFingerprint;
        let key = make_test_key(1);
        assert!(v.verify("example.com", 22, &key).unwrap());
    }

    #[test]
    fn opensshkh_tofu_learns_and_then_accepts() {
        let path = fresh_tmp_path("tofu");
        // Make sure we start clean.
        let _ = fs::remove_file(&path);

        let verifier = HostKeyVerifier::OpenSshKnownHosts { path: path.clone() };
        let key = make_test_key(2);

        // First verify: file doesn't exist → learned + accepted.
        assert!(verifier.verify("first.example.com", 22, &key).unwrap());
        assert!(path.exists(), "TOFU learn must create the file");

        // Second verify for the same host+key: matches → accepted
        // without touching the file.
        assert!(verifier.verify("first.example.com", 22, &key).unwrap());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn opensshkh_mismatch_is_reported_structurally() {
        let path = fresh_tmp_path("mismatch");
        let _ = fs::remove_file(&path);

        let verifier = HostKeyVerifier::OpenSshKnownHosts { path: path.clone() };
        let key_a = make_test_key(3);
        let key_b = make_test_key(4);

        // Learn key_a for the host.
        assert!(verifier.verify("mismatch.example.com", 22, &key_a).unwrap());
        // Now show up with a different key for the same host.
        let err = verifier
            .verify("mismatch.example.com", 22, &key_b)
            .expect_err("mismatch must surface as Err(VerifyError::Mismatch)");
        match err {
            VerifyError::Mismatch {
                host, fingerprint, ..
            } => {
                assert_eq!(host, "mismatch.example.com");
                assert!(
                    fingerprint.contains("SHA256:"),
                    "expected SHA256-prefixed fingerprint, got {fingerprint:?}",
                );
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn default_resolves_to_openssh_path() {
        // The default variant must not be AcceptAll on any
        // supported platform. This catches accidental regressions
        // where someone flips the default "for testing".
        let v = HostKeyVerifier::default();
        assert!(
            matches!(v, HostKeyVerifier::OpenSshKnownHosts { .. }),
            "default must be the real verifier, not {v:?}",
        );
    }
}
