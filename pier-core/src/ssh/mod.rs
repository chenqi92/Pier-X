//! SSH client subsystem.
//!
//! ## Goals
//!
//! Port upstream Pier's SSH + SFTP functionality to Pier-X in a
//! cross-platform, testable shape. The upstream was Unix-only and
//! leaned on `/usr/bin/ssh + ControlMaster`; Pier-X goes
//! russh-only, which means macOS, Windows and Linux all get the
//! same code path with zero fork/exec dependency on a system SSH
//! binary.
//!
//! ## Shape
//!
//! | submodule      | what it owns                                      |
//! |----------------|---------------------------------------------------|
//! | [`config`]     | Plain-data [`config::SshConfig`] + [`config::AuthMethod`] |
//! | [`error`]      | [`error::SshError`] enum wrapping russh + I/O errors |
//! | [`runtime`]    | Process-wide tokio runtime shared by every session |
//! | [`known_hosts`]| Host key verification — parses `~/.ssh/known_hosts` and appends new keys on first connect (OpenSSH `accept-new` / TOFU). `AcceptAllLogFingerprint` stays in the enum for tests. |
//! | [`session`]    | [`session::SshSession`] — connect, authenticate, open channels |
//! | [`channel`]    | [`channel::SshChannelPty`] — implements [`crate::terminal::Pty`] against an SSH interactive channel |
//!
//! ## The critical design payoff
//!
//! M2 designed [`crate::terminal::Pty`] as a send-safe, sync,
//! non-blocking trait. M2a shipped a `UnixPty` backend. M3a ships
//! an `SshChannelPty` backend. Everything above the trait —
//! [`crate::terminal::PierTerminal`] and the Tauri command
//! layer that drives it — targets the trait, NOT any specific
//! backend. Swapping from a local shell to a remote shell is a
//! one-line change:
//!
//! ```ignore
//! // Local (what M2 ships):
//! let pty: Box<dyn Pty> = Box::new(UnixPty::spawn_shell(cols, rows, "/bin/zsh")?);
//! PierTerminal::with_pty(pty, cols, rows, notify, user_data)?;
//!
//! // Remote (what M3 enables):
//! let session = SshSession::connect(&config).await?;
//! let pty: Box<dyn Pty> = Box::new(session.open_shell_channel(cols, rows).await?);
//! PierTerminal::with_pty(pty, cols, rows, notify, user_data)?;
//! ```
//!
//! The sync↔async impedance between a sync `Pty` and russh's async
//! API is absorbed by [`channel::SshChannelPty`]: it owns a tokio
//! task on the shared runtime that drives the russh channel, and
//! exposes a pair of bounded mpsc queues so the sync `read`/`write`
//! calls just push/pop bytes without ever taking a tokio lock.

pub mod channel;
pub mod config;
pub mod error;
pub mod exec_stream;
pub mod known_hosts;
pub mod runtime;
pub mod service_detector;
pub mod session;
pub mod sftp;
pub mod tunnel;

pub use channel::SshChannelPty;
pub use config::{AuthMethod, SshConfig};
pub use error::SshError;
pub use exec_stream::{ExecEvent, ExecStream, EXIT_UNKNOWN};
pub use known_hosts::HostKeyVerifier;
pub use service_detector::{detect_all, detect_all_blocking, DetectedService, ServiceStatus};
pub use session::SshSession;
pub use sftp::{RemoteFileEntry, SftpClient};
pub use tunnel::Tunnel;
