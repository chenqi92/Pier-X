//! Streaming remote exec — the third shape of SSH channel
//! pier-x talks.
//!
//! ## Where this fits
//!
//! `pier-core` already runs SSH channels in three other
//! shapes:
//!
//!   1. [`super::channel::SshChannelPty`] — full interactive
//!      PTY behind a sync `Pty` trait.
//!   2. [`super::sftp::SftpClient`] — request/reply SFTP
//!      subsystem on a fresh channel.
//!   3. [`super::tunnel::Tunnel`] — direct-tcpip channel pair
//!      per inbound connection, plus a bound local listener.
//!
//! The Log viewer panel (M5b) and the Docker panel (M5c) both
//! need a **fourth** shape: spawn a long-running remote
//! command (`tail -f /var/log/syslog`, `docker logs -f <id>`,
//! `docker stats --no-stream=false`, ...), stream its stdout
//! and stderr to the UI line-by-line, and expose a way to
//! terminate it cleanly.
//!
//! This module is the one place that shape lives. `session.rs`
//! exposes [`super::SshSession::spawn_exec_stream`] which
//! opens a channel, sends `exec`, and hands ownership off to
//! [`ExecStream`]. From there the shell polls
//! [`ExecStream::drain`] whenever it's ready to show new
//! events.
//!
//! ## Why std mpsc, not tokio mpsc
//!
//! The consumer lives outside the tokio runtime, so
//! `std::sync::mpsc::Receiver::try_recv` is exactly what we
//! want: non-blocking, thread-safe, zero async glue in the
//! shell command layer. The producer runs inside a `tokio::task`
//! on the shared runtime and just calls `.send`.
//!
//! ## Line framing
//!
//! Remote processes rarely emit exact line boundaries per
//! write. We buffer partial lines per stream (stdout and
//! stderr kept separate) and emit a line as soon as we see a
//! `\n`. The trailing partial line is flushed as its own
//! event when the channel closes — so `echo -n "no newline"`
//! still shows up in the viewer on exit.
//!
//! ## Backpressure
//!
//! The shell pulls lazily via `drain`, so a pathological remote
//! can buffer unbounded bytes. Log viewer caps what it
//! actually retains in the model (see `PierLogStream`), but
//! the channel itself is unbounded here because
//! `try_recv`-based consumers can't offer a bounded queue
//! without sometimes dropping events — and dropping log
//! lines silently is a worse bug than a large buffer.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use super::error::Result;
use super::runtime;
use super::session::SshSession;

/// Exit-code sentinel meaning "the remote didn't report one"
/// (channel closed without an ExitStatus message).
pub const EXIT_UNKNOWN: i32 = -1;

/// One unit of output from a streamed exec. Emitted by the
/// tokio-side producer and consumed by the UI via
/// [`ExecStream::drain`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecEvent {
    /// A complete line of stdout (no trailing `\n`).
    Stdout(String),
    /// A complete line of stderr (no trailing `\n`).
    Stderr(String),
    /// Remote process exited. Always the last event emitted
    /// before the channel closes.
    Exit(i32),
    /// Transport-level error. After this, no further events
    /// will arrive.
    Error(String),
}

impl ExecEvent {
    /// Short tag for JSON / logging. Stable across releases.
    pub fn kind(&self) -> &'static str {
        match self {
            ExecEvent::Stdout(_) => "stdout",
            ExecEvent::Stderr(_) => "stderr",
            ExecEvent::Exit(_) => "exit",
            ExecEvent::Error(_) => "error",
        }
    }
}

/// Handle to a running remote exec. Cheap to move, not
/// clonable — each stream owns its receiver exclusively.
///
/// Dropping this handle triggers a best-effort cancel
/// (see [`Self::stop`]).
pub struct ExecStream {
    rx: Receiver<ExecEvent>,
    stop_flag: Arc<AtomicBool>,
    exit_code: Arc<AtomicI32>,
    finished: Arc<AtomicBool>,
}

impl std::fmt::Debug for ExecStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecStream")
            .field("finished", &self.finished.load(Ordering::Acquire))
            .field("exit_code", &self.exit_code.load(Ordering::Acquire))
            .finish()
    }
}

impl ExecStream {
    /// Pop every event currently available without blocking.
    /// An empty return does **not** mean the stream has ended
    /// — check [`Self::is_alive`] for that. Call pattern on
    /// the UI side is typically a 50–250 ms `QTimer`.
    pub fn drain(&self) -> Vec<ExecEvent> {
        self.drain_up_to(usize::MAX).0
    }

    /// Pop up to `limit` currently-buffered events without blocking
    /// and report whether the receiver was fully drained afterward.
    ///
    /// `exhausted == false` means we stopped because the limit was
    /// reached, so more buffered events may still be waiting.
    pub fn drain_up_to(&self, limit: usize) -> (Vec<ExecEvent>, bool) {
        let mut out = Vec::new();
        loop {
            if out.len() >= limit {
                return (out, false);
            }
            match self.rx.try_recv() {
                Ok(ev) => out.push(ev),
                Err(TryRecvError::Empty) => return (out, true),
                Err(TryRecvError::Disconnected) => {
                    // Producer is gone. Make sure the finished
                    // flag is set so is_alive reports correctly,
                    // even if the producer didn't get to set it
                    // itself (panic path).
                    self.finished.store(true, Ordering::Release);
                    return (out, true);
                }
            }
        }
    }

    /// True while the producer task is still running AND no
    /// [`ExecEvent::Exit`] / [`ExecEvent::Error`] has been
    /// observed yet.
    pub fn is_alive(&self) -> bool {
        !self.finished.load(Ordering::Acquire)
    }

    /// Last reported exit code, or [`EXIT_UNKNOWN`] if the
    /// remote process hasn't reported one yet.
    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Acquire)
    }

    /// Flip the stop flag. The producer task checks this
    /// between each `channel.wait()`, closes the channel on
    /// the next iteration, and drops off. Note that SSH
    /// itself has no general way to signal the remote process
    /// — closing the channel makes the remote see a SIGPIPE
    /// on its next write, which is what `tail -f` / `docker
    /// logs -f` both honor.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::Release);
    }
}

impl Drop for ExecStream {
    fn drop(&mut self) {
        self.stop();
    }
}

impl SshSession {
    /// Spawn `command` on the remote, returning a streaming
    /// [`ExecStream`] handle. The channel is opened on the
    /// calling task; the producer loop that reads events runs
    /// on the shared tokio runtime.
    ///
    /// This is a low-level primitive: the caller passes the
    /// exact command string the server will run via its login
    /// shell. Callers that care about shell quoting should
    /// apply their own escaping.
    pub async fn spawn_exec_stream(&self, command: &str) -> Result<ExecStream> {
        let mut channel = self.handle_arc().channel_open_session().await?;
        channel.exec(true, command).await?;

        let (tx, rx) = mpsc::channel::<ExecEvent>();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let exit_code = Arc::new(AtomicI32::new(EXIT_UNKNOWN));

        let stop_for_task = Arc::clone(&stop_flag);
        let finished_for_task = Arc::clone(&finished);
        let exit_for_task = Arc::clone(&exit_code);

        // Producer task. Owns the channel; when it exits the
        // channel is dropped, which russh teardowns the remote
        // side. We deliberately do *not* spawn this on the
        // calling future's runtime — we always use the shared
        // runtime so cancelling the caller never kills the
        // exec stream.
        runtime::shared().spawn(async move {
            let mut stdout_buf: Vec<u8> = Vec::with_capacity(4096);
            let mut stderr_buf: Vec<u8> = Vec::with_capacity(1024);

            loop {
                if stop_for_task.load(Ordering::Acquire) {
                    break;
                }
                let msg = match channel.wait().await {
                    Some(m) => m,
                    None => break,
                };
                match msg {
                    russh::ChannelMsg::Data { data } => {
                        stdout_buf.extend_from_slice(&data);
                        drain_lines(
                            &mut stdout_buf,
                            &tx,
                            ExecEvent::Stdout as fn(String) -> ExecEvent,
                        );
                    }
                    russh::ChannelMsg::ExtendedData { data, ext } => {
                        // ext == 1 is stderr in RFC 4254. Any
                        // other ext id is a vendor extension
                        // we don't understand; route it to
                        // stderr too rather than dropping it.
                        let _ = ext;
                        stderr_buf.extend_from_slice(&data);
                        drain_lines(
                            &mut stderr_buf,
                            &tx,
                            ExecEvent::Stderr as fn(String) -> ExecEvent,
                        );
                    }
                    russh::ChannelMsg::ExitStatus { exit_status } => {
                        exit_for_task.store(exit_status as i32, Ordering::Release);
                    }
                    russh::ChannelMsg::Eof | russh::ChannelMsg::Close => {
                        // Match `SshSession::exec_command()`: a
                        // few servers send Close before the final
                        // ExitStatus, so keep draining until
                        // `channel.wait()` returns None.
                    }
                    _ => {}
                }
            }

            // Flush any trailing partial line so `echo -n` etc.
            // still surface before we announce Exit.
            flush_partial(&mut stdout_buf, &tx, ExecEvent::Stdout);
            flush_partial(&mut stderr_buf, &tx, ExecEvent::Stderr);

            let code = exit_for_task.load(Ordering::Acquire);
            let _ = tx.send(ExecEvent::Exit(code));
            finished_for_task.store(true, Ordering::Release);
            // tx drops here → Receiver::try_recv will return
            // Disconnected once it's drained the channel.
        });

        Ok(ExecStream {
            rx,
            stop_flag,
            exit_code,
            finished,
        })
    }

    /// Synchronous wrapper for [`Self::spawn_exec_stream`].
    pub fn spawn_exec_stream_blocking(&self, command: &str) -> Result<ExecStream> {
        runtime::shared().block_on(self.spawn_exec_stream(command))
    }
}

/// Pull complete lines (ending in `\n`) out of `buf`, decoding
/// each as UTF-8 with lossy fallback, wrapping in `ctor`, and
/// sending through `tx`. Partial tail remains in `buf`.
fn drain_lines(buf: &mut Vec<u8>, tx: &mpsc::Sender<ExecEvent>, ctor: fn(String) -> ExecEvent) {
    loop {
        let Some(nl_idx) = buf.iter().position(|&b| b == b'\n') else {
            return;
        };
        // Take bytes [0..nl_idx) — the `\n` itself is dropped,
        // a trailing `\r` (CRLF) is trimmed too.
        let mut line: Vec<u8> = buf.drain(..=nl_idx).collect();
        line.pop(); // the '\n'
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        let text = String::from_utf8_lossy(&line).into_owned();
        if tx.send(ctor(text)).is_err() {
            // Receiver is gone — just swallow. The consumer
            // stopped listening, and we'll exit on the next
            // loop iteration via the stop flag or channel close.
            return;
        }
    }
}

/// Flush any remaining bytes in `buf` as a single (possibly
/// newline-less) line, then clear the buffer.
fn flush_partial(buf: &mut Vec<u8>, tx: &mpsc::Sender<ExecEvent>, ctor: fn(String) -> ExecEvent) {
    if buf.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(buf).into_owned();
    buf.clear();
    let _ = tx.send(ctor(text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn drain_lines_splits_on_newline() {
        let (tx, rx) = channel::<ExecEvent>();
        let mut buf: Vec<u8> = b"hello\nworld\n".to_vec();
        drain_lines(&mut buf, &tx, ExecEvent::Stdout);
        drop(tx);
        assert!(buf.is_empty());
        let events: Vec<_> = rx.iter().collect();
        assert_eq!(
            events,
            vec![
                ExecEvent::Stdout("hello".into()),
                ExecEvent::Stdout("world".into()),
            ]
        );
    }

    #[test]
    fn drain_lines_keeps_partial_tail() {
        let (tx, rx) = channel::<ExecEvent>();
        let mut buf: Vec<u8> = b"first\nstill typing".to_vec();
        drain_lines(&mut buf, &tx, ExecEvent::Stdout);
        drop(tx);
        assert_eq!(buf, b"still typing");
        let events: Vec<_> = rx.iter().collect();
        assert_eq!(events, vec![ExecEvent::Stdout("first".into())]);
    }

    #[test]
    fn drain_lines_trims_crlf() {
        let (tx, rx) = channel::<ExecEvent>();
        let mut buf: Vec<u8> = b"windows\r\nline\r\n".to_vec();
        drain_lines(&mut buf, &tx, ExecEvent::Stderr);
        drop(tx);
        let events: Vec<_> = rx.iter().collect();
        assert_eq!(
            events,
            vec![
                ExecEvent::Stderr("windows".into()),
                ExecEvent::Stderr("line".into()),
            ]
        );
    }

    #[test]
    fn flush_partial_emits_tail_if_any() {
        let (tx, rx) = channel::<ExecEvent>();
        let mut buf: Vec<u8> = b"no newline here".to_vec();
        flush_partial(&mut buf, &tx, ExecEvent::Stdout);
        drop(tx);
        assert!(buf.is_empty());
        let events: Vec<_> = rx.iter().collect();
        assert_eq!(events, vec![ExecEvent::Stdout("no newline here".into())]);
    }

    #[test]
    fn flush_partial_empty_is_noop() {
        let (tx, rx) = channel::<ExecEvent>();
        let mut buf: Vec<u8> = Vec::new();
        flush_partial(&mut buf, &tx, ExecEvent::Stdout);
        drop(tx);
        let events: Vec<_> = rx.iter().collect();
        assert!(events.is_empty());
    }

    #[test]
    fn drain_lines_replaces_invalid_utf8_lossily() {
        let (tx, rx) = channel::<ExecEvent>();
        // 0xC3 0x28 is the canonical example of an invalid
        // two-byte UTF-8 sequence. `from_utf8_lossy` turns it
        // into U+FFFD plus the '('.
        let mut buf: Vec<u8> = vec![0xC3, 0x28, b'\n'];
        drain_lines(&mut buf, &tx, ExecEvent::Stdout);
        drop(tx);
        let events: Vec<_> = rx.iter().collect();
        assert_eq!(events, vec![ExecEvent::Stdout("\u{FFFD}(".into())]);
    }

    #[test]
    fn exec_event_kind_tag_is_stable() {
        assert_eq!(ExecEvent::Stdout("".into()).kind(), "stdout");
        assert_eq!(ExecEvent::Stderr("".into()).kind(), "stderr");
        assert_eq!(ExecEvent::Exit(0).kind(), "exit");
        assert_eq!(ExecEvent::Error("x".into()).kind(), "error");
    }

    #[test]
    fn drop_sets_stop_flag() {
        // We can construct an ExecStream with a dummy pair
        // (no real SSH channel) to exercise the drop path.
        let (_tx, rx) = mpsc::channel::<ExecEvent>();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let exit = Arc::new(AtomicI32::new(EXIT_UNKNOWN));
        let stop_clone = Arc::clone(&stop_flag);
        let stream = ExecStream {
            rx,
            stop_flag,
            exit_code: exit,
            finished,
        };
        assert!(!stop_clone.load(Ordering::Acquire));
        drop(stream);
        assert!(stop_clone.load(Ordering::Acquire));
    }

    #[test]
    fn is_alive_flips_when_finished_flag_set() {
        let (_tx, rx) = mpsc::channel::<ExecEvent>();
        let stream = ExecStream {
            rx,
            stop_flag: Arc::new(AtomicBool::new(false)),
            exit_code: Arc::new(AtomicI32::new(EXIT_UNKNOWN)),
            finished: Arc::new(AtomicBool::new(false)),
        };
        assert!(stream.is_alive());
        stream.finished.store(true, Ordering::Release);
        assert!(!stream.is_alive());
    }
}
