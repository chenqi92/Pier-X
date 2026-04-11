//! Terminal subsystem — PTY process spawning + VT100/ANSI emulation.
//!
//! This module is the engine behind the local terminal feature. It is
//! organized into two layers that compose cleanly and can be tested
//! independently:
//!
//! * [`pty`] — owns the child process. Exposes a byte-oriented
//!   `Pty` trait with `read`, `write`, `resize`, and a destructor that
//!   reaps the child. A `UnixPty` implementation wraps `forkpty(3)` on
//!   Unix targets; Windows gets a `WindowsPty` stub that currently
//!   returns [`TerminalError::Unsupported`] and will be filled in with
//!   the Win32 **ConPTY** API (`CreatePseudoConsole`) in M2b.
//!
//! * [`emulator`] — pure-Rust VT100 state machine, driven by the `vte`
//!   crate's SAX-style `Perform` trait. Holds a rectangular grid of
//!   [`emulator::Cell`]s that the UI paints, a cursor position, and
//!   honours a minimum-viable set of CSI sequences (cursor movement,
//!   erase in display / in line). Colors and SGR attributes are parsed
//!   but not yet applied — the plumbing is there for M2b to enable
//!   without touching this file.
//!
//! These two layers intentionally do NOT know about each other.
//! `pty::Pty` produces raw bytes, `emulator::VtEmulator` consumes raw
//! bytes — the code that wires them together lives one layer up, in
//! the UI-facing `TerminalSession` that M2b will introduce alongside
//! the Qt bridge. Keeping them separate means:
//!
//! 1. The emulator tests don't need a real shell. They feed canned
//!    byte sequences and assert grid contents.
//! 2. A future remote-PTY implementation backed by an SSH channel
//!    drops into the same `Pty` trait and reuses the emulator.

pub mod emulator;
pub mod pty;
pub mod session;

pub use emulator::{Cell, Color, VtEmulator};
pub use pty::{Pty, TerminalError};
pub use session::{GridSnapshot, NotifyEvent, NotifyFn, PierTerminal};

#[cfg(unix)]
pub use pty::UnixPty;

#[cfg(windows)]
pub use pty::WindowsPty;
