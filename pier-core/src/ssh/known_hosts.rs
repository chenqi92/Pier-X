//! Host key verification.
//!
//! ## What exists in M3a
//!
//! A placeholder [`HostKeyVerifier`] that accepts every server
//! key it sees. This is **not** what production pier-x will use —
//! it exists so the rest of the SSH module can be wired up and
//! tested on real servers during development without also having
//! to build the full known-hosts parser at the same time.
//!
//! The verifier runs behind an enum so the accept-all branch is
//! obviously labeled at every call site, and so the M3b swap-in
//! of real OpenSSH-compatible known_hosts parsing is a one-file
//! change that doesn't ripple through `session.rs`.
//!
//! ## What M3b will add
//!
//! * [`HostKeyVerifier::OpenSshKnownHosts`] that reads
//!   `~/.ssh/known_hosts` on connect, handles hashed hosts, the
//!   `@cert-authority` marker, `revoked` lines, and produces a
//!   proper SHA-256 fingerprint on mismatch.
//! * A UI pipeline: `HostKeyMismatch` surfaces to the user with
//!   "trust this fingerprint?" and writes the answer back.
//! * Optional strict mode that refuses any first-connection
//!   without a pre-pinned fingerprint.
//!
//! Until then, the upstream Pier behavior (accept on first
//! connect, warn later) is what Pier-X ships in M3a. We log the
//! fingerprint of every accepted key so a user can still inspect
//! what they trusted.

use russh::keys::ssh_key::PublicKey;

/// How an SSH session decides whether to trust a server's host key.
#[derive(Debug, Clone, Default)]
pub enum HostKeyVerifier {
    /// M3a default: accept any server key, log the fingerprint.
    /// Do NOT ship this to end users as-is — M3b replaces it with
    /// [`HostKeyVerifier::OpenSshKnownHosts`].
    #[default]
    AcceptAllLogFingerprint,
    // OpenSshKnownHosts { path: PathBuf }  // M3b
    // PinnedFingerprints { map: HashMap<String, String> }  // M3b
}

impl HostKeyVerifier {
    /// Verify a server-presented key for the given host. Returns
    /// `Ok(true)` to accept, `Ok(false)` to reject, `Err(_)` on
    /// parse / I/O failure reading a known_hosts file.
    ///
    /// Called from inside russh's async `check_server_key` handler.
    pub fn verify(&self, host: &str, key: &PublicKey) -> Result<bool, std::io::Error> {
        match self {
            Self::AcceptAllLogFingerprint => {
                let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256);
                log::info!(
                    "ssh host key for {host} (accepted, M3a verifier): {fingerprint}",
                );
                Ok(true)
            }
        }
    }
}

