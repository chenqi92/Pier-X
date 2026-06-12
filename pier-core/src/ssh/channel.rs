//! [`SshChannelPty`] — sync [`crate::terminal::Pty`] over an async
//! russh channel.
//!
//! ## The architectural payoff
//!
//! M2 designed [`crate::terminal::Pty`] as:
//!
//!  * object-safe (`Box<dyn Pty>` is fine)
//!  * `Send` (can move across thread boundaries)
//!  * synchronous, non-blocking `read` (returns empty vec when no
//!    data rather than blocking)
//!  * synchronous `write` / `resize` with immediate completion
//!
//! None of that precludes an async backend — it just means the
//! backend has to own its own task and expose sync-looking
//! methods that communicate with the task via queues. That is
//! exactly what this file does. The result is that *every line
//! of code above the `Pty` trait* (session layer, terminal
//! runtime, shell surface, keyboard routing) is agnostic to
//! whether bytes are coming from a local `forkpty` child or
//! from a remote shell over a russh channel.
//!
//! ## Layout
//!
//! ```text
//!   sync caller                 tokio task (on shared runtime)
//!   ───────────                 ──────────────────────────────
//!   Pty::write(bytes) ───────►  ControlMsg::Write(bytes)
//!                                │
//!                                ▼
//!                         channel.data(bytes).await
//!   Pty::resize(c,r)  ───────►  ControlMsg::Resize { c, r }
//!                                │
//!                                ▼
//!                         channel.window_change(c,r).await
//!                                │
//!                                ▼
//!                         channel.wait() → ChannelMsg::Data
//!                                │
//!                                ▼
//!                         data_tx.send(bytes)
//!   Pty::read()      ◄───────  data_rx.try_recv()
//!
//!   drop              ───────►  ControlMsg::Close → task aborts
//! ```
//!
//! Both queues are `tokio::sync::mpsc::unbounded_channel`s. The
//! sync side uses `try_send` / `try_recv` so it never blocks —
//! if the reader thread upstairs doesn't drain fast enough, the
//! queue grows, which is fine because terminal output is bursty
//! and bounded by human reading speed in the long run. If this
//! becomes a memory concern later, swap to `bounded(N)` here —
//! no caller needs to change.

use std::sync::Mutex;

use russh::ChannelMsg;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use super::runtime;
use crate::terminal::{Pty, TerminalError};

/// Commands the sync side sends to the async task driving the
/// russh channel. Close is implicit via dropping the sender.
enum ControlMsg {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

/// A [`Pty`] implementation backed by a russh interactive channel.
///
/// Construct via [`super::SshSession::open_shell_channel`] — do
/// NOT try to build one of these directly; the russh channel it
/// wraps must already have had `request_pty` and `request_shell`
/// called on it.
pub struct SshChannelPty {
    control_tx: UnboundedSender<ControlMsg>,
    // Mutex only because std::sync::mpsc::Receiver is !Sync;
    // the lock is uncontended in practice (only Pty::read holds it).
    data_rx: Mutex<UnboundedReceiver<Vec<u8>>>,
    task: Option<JoinHandle<()>>,
    cols: u16,
    rows: u16,
}

impl SshChannelPty {
    /// Spawn the background task that owns the russh channel and
    /// wire up the two bridge queues. This is only called from
    /// [`super::SshSession::open_shell_channel`] — every other
    /// constructor path is either for tests or future protocols.
    pub(super) fn spawn(
        channel: russh::Channel<russh::client::Msg>,
        cols: u16,
        rows: u16,
        prelude: Vec<u8>,
    ) -> Self {
        let (control_tx, control_rx) = unbounded_channel::<ControlMsg>();
        let (data_tx, data_rx) = unbounded_channel::<Vec<u8>>();

        // Bytes the shell emitted before its channel was confirmed (see
        // SshSession::confirm_shell_started) are replayed ahead of the
        // live stream so the first prompt is never dropped.
        if !prelude.is_empty() {
            let _ = data_tx.send(prelude);
        }

        // Handoff task: drives the russh channel, fans bytes into
        // data_tx, reacts to ControlMsg from the sync side.
        let task = runtime::shared().spawn(async move {
            channel_loop(channel, control_rx, data_tx).await;
        });

        Self {
            control_tx,
            data_rx: Mutex::new(data_rx),
            task: Some(task),
            cols,
            rows,
        }
    }
}

/// The async half of the bridge. Stays alive until either the
/// control queue is dropped (sync side stopped) or the russh
/// channel emits `Close` / `Eof`.
async fn channel_loop(
    mut channel: russh::Channel<russh::client::Msg>,
    mut control_rx: UnboundedReceiver<ControlMsg>,
    data_tx: UnboundedSender<Vec<u8>>,
) {
    // Recorded so the single exit log below names the cause. A silent
    // exit is exactly what made "ssh channel task has exited" so hard
    // to diagnose — every break now stamps why.
    let reason: &str;
    loop {
        tokio::select! {
            biased;

            // ── Sync side → async side ────────────────────────
            cmd = control_rx.recv() => {
                match cmd {
                    Some(ControlMsg::Write(bytes)) => {
                        if let Err(e) = channel.data(&bytes[..]).await {
                            log::warn!("ssh channel write failed: {e}");
                            reason = "write error";
                            break;
                        }
                    }
                    Some(ControlMsg::Resize { cols, rows }) => {
                        if let Err(e) = channel
                            .window_change(cols as u32, rows as u32, 0, 0)
                            .await
                        {
                            log::warn!("ssh channel resize failed: {e}");
                            reason = "resize error";
                            break;
                        }
                    }
                    None => {
                        // Sync side dropped the PTY — close the
                        // channel cleanly and exit.
                        let _ = channel.eof().await;
                        let _ = channel.close().await;
                        reason = "pty dropped";
                        break;
                    }
                }
            }

            // ── Async side → sync side ────────────────────────
            msg = channel.wait() => {
                match msg {
                    Some(ChannelMsg::Data { data }) => {
                        if data_tx.send(data.to_vec()).is_err() {
                            // Sync side stopped reading — nothing
                            // left to do.
                            reason = "reader gone";
                            break;
                        }
                    }
                    Some(ChannelMsg::ExtendedData { data, ext }) => {
                        // ext == 1 is stderr. For an interactive
                        // shell we fold stderr into the same byte
                        // stream so the terminal emulator sees it
                        // in order — matches what every other
                        // terminal emulator does.
                        if ext == 1 && data_tx.send(data.to_vec()).is_err() {
                            reason = "reader gone";
                            break;
                        }
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) => {
                        reason = "server closed channel";
                        break;
                    }
                    Some(ChannelMsg::ExitStatus { .. }) => {
                        // Exit status arrives before Close. Log
                        // and keep draining until Close/Eof lands.
                    }
                    Some(_) => {
                        // Other message kinds (OpenFailure,
                        // Success, Failure, WindowAdjusted, ...)
                        // are handled internally by russh; nothing
                        // for us to do.
                    }
                    None => {
                        reason = "transport gone";
                        break;
                    }
                }
            }
        }
    }
    log::info!("ssh channel task exiting: {reason}");
}

impl Pty for SshChannelPty {
    fn read(&mut self) -> Result<Vec<u8>, TerminalError> {
        use tokio::sync::mpsc::error::TryRecvError;
        // Coalesce whatever's been queued since the last call
        // into one Vec so the emulator feeds it in a single
        // `process` call (which is slightly more efficient than
        // one call per chunk).
        let mut out = Vec::new();
        let mut guard = self.data_rx.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            match guard.try_recv() {
                Ok(chunk) => out.extend_from_slice(&chunk),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // The channel task exited (server closed the
                    // channel / session died). Deliver any tail
                    // bytes first; the next read reports EOF so the
                    // reader thread fires Exited instead of polling
                    // a dead channel forever with the tab stuck on
                    // "running".
                    if out.is_empty() {
                        return Err(TerminalError::Io(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "ssh channel closed",
                        )));
                    }
                    break;
                }
            }
        }
        Ok(out)
    }

    fn write(&mut self, data: &[u8]) -> Result<usize, TerminalError> {
        if self
            .control_tx
            .send(ControlMsg::Write(data.to_vec()))
            .is_err()
        {
            return Err(TerminalError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "ssh channel task has exited",
            )));
        }
        // Unbounded mpsc.send always accepts the full payload, so
        // we report "all bytes written" on success. A future move
        // to bounded(N) would return the amount actually queued.
        Ok(data.len())
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        self.control_tx
            .send(ControlMsg::Resize { cols, rows })
            .map_err(|_| {
                TerminalError::Io(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "ssh channel task has exited",
                ))
            })?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
}

impl Drop for SshChannelPty {
    fn drop(&mut self) {
        // Dropping the sender wakes the channel loop on its
        // control_rx.recv() branch with `None`, which runs the clean
        // shutdown (eof + close) and exits.
        //
        // Crucially we do NOT abort the task. `JoinHandle::abort` cancels
        // the future at its next await point, which races — and beats —
        // the in-flight `channel.eof().await` / `channel.close().await`,
        // so the SSH CHANNEL_CLOSE never reaches the server and the
        // server-side session slot leaks. Over many terminal open/close
        // cycles that exhausts the server's MaxSessions, and new shell
        // channels are then refused with "Failed to open channel
        // (ConnectFailed)" even though the connection (and SFTP on it) is
        // fine. Dropping the JoinHandle simply detaches the task; it
        // finishes eof+close on the shared runtime — both return promptly
        // even on a dead link (the send to the transport task just errors)
        // — and ends on its own.
        drop(std::mem::replace(
            &mut self.control_tx,
            unbounded_channel().0,
        ));
        self.task.take();
    }
}
