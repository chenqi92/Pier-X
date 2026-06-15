//! Error type for the remote-desktop (RDP / VNC) backends.

use std::io;

/// Failure modes shared by the RDP and VNC clients.
#[derive(Debug, thiserror::Error)]
pub enum RemoteDesktopError {
    /// Underlying socket / I/O failure.
    #[error("remote desktop I/O: {0}")]
    Io(#[from] io::Error),

    /// TCP connect / TLS handshake failed before the protocol started.
    #[error("connect failed: {0}")]
    Connect(String),

    /// The server rejected our credentials, or we could not satisfy the
    /// security handshake (e.g. an unsupported VNC security type).
    #[error("authentication failed: {0}")]
    Auth(String),

    /// The peer sent something we could not parse / did not expect.
    #[error("protocol error: {0}")]
    Protocol(String),

    /// A capability the server demanded (or we were asked to use) is not
    /// implemented yet — surfaced verbatim to the user.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Convenience alias used throughout the `remote_desktop` module.
pub type Result<T> = std::result::Result<T, RemoteDesktopError>;
