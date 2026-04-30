//! SSH subsystem error type.
//!
//! Wraps `russh::Error`, `std::io::Error`, and a handful of
//! pier-core-specific conditions (auth rejected, host key
//! mismatch, channel closed) into one enum the app runtime can
//! map into user-facing errors.

use std::io;

/// Result alias for SSH operations.
pub type Result<T> = std::result::Result<T, SshError>;

/// Every way an SSH operation inside pier-core can fail.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// TCP connect or DNS lookup failed before the SSH handshake
    /// could start.
    #[error("ssh connect failed: {0}")]
    Connect(#[source] io::Error),

    /// The SSH library itself raised an error during handshake,
    /// authentication, channel open, or data transfer.
    #[error("ssh protocol: {0}")]
    Protocol(#[from] russh::Error),

    /// Authentication failed — the server rejected every method
    /// we tried.
    #[error("ssh authentication rejected (tried: {tried:?})")]
    AuthRejected {
        /// Human-readable summary of which methods we attempted.
        /// Used by the UI to tell the user "tried password + key,
        /// both rejected".
        tried: Vec<String>,
    },

    /// The remote host key didn't match our pinned entry in
    /// known_hosts. Not fatal by itself — the UI can prompt the
    /// user to approve the new fingerprint.
    #[error("ssh host key mismatch for {host}: got {fingerprint}")]
    HostKeyMismatch {
        /// Hostname the user tried to connect to.
        host: String,
        /// SHA-256 fingerprint of the key the remote actually
        /// presented, in the `SHA256:abcd...` form OpenSSH prints.
        fingerprint: String,
    },

    /// The channel we were reading from or writing to has closed.
    /// Almost always means the remote shell exited.
    #[error("ssh channel closed")]
    ChannelClosed,

    /// Connection timed out waiting for TCP / handshake.
    #[error("ssh connect timeout after {0:?}")]
    Timeout(std::time::Duration),

    /// Configuration was missing required fields
    /// ([`super::SshConfig::is_valid`] returned false).
    #[error("invalid ssh config: {0}")]
    InvalidConfig(String),

    /// Any other I/O error (reading a key file, talking to the
    /// keyring, etc.).
    #[error("ssh i/o: {0}")]
    Io(#[from] io::Error),

    /// The caller tried to use an SSH session after dropping it,
    /// or passed a stale channel handle. Should be unreachable
    /// under normal use.
    #[error("ssh session is no longer alive")]
    Dead,

    /// A long-running transfer was aborted by the user via the
    /// frontend's cancel command. The destination file is left in
    /// its partial state — re-running the transfer with the same
    /// transfer-id picks up where this one stopped (the auto-resume
    /// machinery in `sftp.rs` reads the destination size on retry).
    #[error("transfer cancelled")]
    Cancelled,
}
