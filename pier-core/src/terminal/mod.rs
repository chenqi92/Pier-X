//! Terminal subsystem — PTY process spawning + VT100/ANSI emulation.
//!
//! This module is the engine behind the local terminal feature. It is
//! organized into two layers that compose cleanly and can be tested
//! independently:
//!
//! * [`pty`] — owns the child process. Exposes a byte-oriented
//!   `Pty` trait with `read`, `write`, `resize`, and a destructor that
//!   reaps the child. A `UnixPty` wraps `forkpty(3)` on Unix; a
//!   `WindowsPty` wraps `CreatePseudoConsole` (ConPTY) on Windows.
//!   Both backends produce the same VT byte stream so the emulator
//!   above them doesn't know which OS it is running on.
//!
//! * [`emulator`] — pure-Rust VT100 state machine, driven by the `vte`
//!   crate's SAX-style `Perform` trait. Holds a rectangular grid of
//!   [`emulator::Cell`]s that the shell paints, a cursor position, and
//!   honours cursor movement, erase-in-display / in-line, plus SGR
//!   colors (16-color + 256-color + true-color), bold, underline, and
//!   reverse-video.
//!
//! These two layers intentionally do NOT know about each other.
//! `pty::Pty` produces raw bytes, `emulator::VtEmulator` consumes raw
//! bytes — the code that wires them together lives one layer up in
//! the shell-facing terminal session. Keeping them separate means:
//!
//! 1. The emulator tests don't need a real shell. They feed canned
//!    byte sequences and assert grid contents.
//! 2. A future remote-PTY implementation backed by an SSH channel
//!    drops into the same `Pty` trait and reuses the emulator.

pub mod completions;
pub mod emulator;
pub mod history;
pub mod man;
pub mod pty;
pub mod session;
pub mod smart;
pub mod ssh_watcher;
pub mod validate;

pub use completions::{complete, Completion, CompletionKind};
pub use emulator::{Cell, Color, VtEmulator};
pub use history::{
    append as history_append, clear as history_clear, is_sensitive,
    load as history_load, HistoryError,
};
pub use man::{man_synopsis, ManError, ManOption, ManSynopsis};
pub use pty::{Pty, TerminalError};
pub use session::{GridSnapshot, NotifyEvent, NotifyFn, PierTerminal};
pub use smart::{inject_init, SmartShellInit};
pub use ssh_watcher::SshChildTarget;
pub use validate::{validate_command, CommandKind};

#[cfg(unix)]
pub use pty::UnixPty;

#[cfg(windows)]
pub use pty::WindowsPty;
