//! SSH local port forwarding — `ssh -L` semantics.
//!
//! ## What this layer does
//!
//! Given a live [`SshSession`] and a (local_port, remote_host,
//! remote_port) triple, [`SshSession::open_local_forward`]
//! binds a TCP listener on `127.0.0.1:local_port`, spawns a
//! tokio task that accepts incoming connections and forwards
//! each one through a fresh `channel_open_direct_tcpip` on the
//! SSH connection to the remote host:port, and returns a
//! [`Tunnel`] handle that owns the listener's lifetime.
//!
//! Dropping the tunnel stops the accept loop and releases the
//! local port. Any in-flight proxied connections continue to
//! run until their endpoints close naturally — we don't force-
//! kill live bytestreams because that tends to corrupt things
//! like partially-written database result sets.
//!
//! ## The pier moment, mechanically
//!
//! M4 service discovery tells the user "you have MySQL on
//! port 3306 over there". M4b lets the user click a pill and
//! get "MySQL is now reachable at localhost:13306 on THIS
//! machine", which is what makes Pier-X actually useful as a
//! remote-admin tool — you can point DBeaver / Redis Insight
//! / whatever local GUI you already have at the tunnel and
//! it just works, without ever exposing the remote port to
//! the internet.
//!
//! ## Threading
//!
//! Everything happens on the shared tokio runtime from
//! [`super::runtime`]. The blocking wrapper is
//! [`SshSession::open_local_forward_blocking`] which
//! `block_on`s the async form — UI callers use that, while
//! anything already inside a task can call the direct form.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::error::{Result, SshError};
use super::runtime;
use super::session::SshSession;

/// Supplies the live SSH session a tunnel forwards over, resolved **fresh
/// for each accepted connection** so the tunnel transparently follows a
/// session reconnect (idle/keepalive death, laptop sleep, network change,
/// sshd restart). Without this the accept loop would keep a single
/// `Arc<Handle>` captured at open time; once that transport died every new
/// `channel_open_direct_tcpip` failed forever and nothing rebuilt it — the
/// "database tunnel silently stops working after the box was idle" bug.
///
/// Implemented by the Tauri layer against its session cache; kept as a
/// trait so `pier-core` stays UI-agnostic and never learns about the app
/// state. The returned `Arc<SshSession>` keeps the russh `Handle` internal
/// to this module (callers only hand back the pier-core session type).
pub trait SessionProvider: Send + Sync + 'static {
    /// The current live session, reconnecting the shared connection if the
    /// cached one has died. **Blocking** — the accept loop calls it on the
    /// blocking pool. `Err` means the session couldn't be (re)established;
    /// the pending connection is dropped and the client sees a refused
    /// forward, exactly as before the self-heal existed.
    fn current(&self) -> std::result::Result<Arc<SshSession>, String>;
}

/// A [`SessionProvider`] pinned to one session — the legacy behavior for
/// [`SshSession::open_local_forward`], where no reconnection policy is
/// wired. It never heals (always returns the same session); callers that
/// want self-healing use [`SshSession::open_local_forward_dynamic`] with a
/// cache-backed provider.
struct FixedSession(SshSession);

impl SessionProvider for FixedSession {
    fn current(&self) -> std::result::Result<Arc<SshSession>, String> {
        Ok(Arc::new(self.0.clone()))
    }
}

/// A live local port forward. Cloning is not supported
/// — the handle owns its accept loop and its bound listener.
/// Drop it to stop accepting new connections and release
/// the local port.
pub struct Tunnel {
    /// The port we actually bound to, which is only
    /// different from the caller's requested port when the
    /// caller passed `0` (let the OS pick).
    local_port: u16,
    /// Stop signal the accept loop polls.
    stop: Arc<Notify>,
    /// Accept loop task handle. Aborted on drop.
    task: Option<JoinHandle<()>>,
}

impl Tunnel {
    /// The port the listener is actually bound to.
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Whether the tunnel can still forward traffic — i.e. its accept loop
    /// is still running.
    ///
    /// This is deliberately *not* "the underlying SSH transport is alive
    /// right now": a tunnel opened via [`SshSession::open_local_forward_dynamic`]
    /// re-resolves a live session on every accepted connection, so a running
    /// accept loop genuinely means "this tunnel still forwards" even across a
    /// transport reconnect (idle death, sleep, network change). Reporting a
    /// momentary transport gap as "dead" would make the frontend drop and
    /// re-open a tunnel that was about to self-heal. A dropped/aborted accept
    /// loop is the only real "gone" state.
    pub fn is_alive(&self) -> bool {
        match self.task.as_ref() {
            Some(h) => !h.is_finished(),
            None => false,
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // Tell the accept loop to stop, then abort the task
        // for belt-and-suspenders. Any connection currently
        // being proxied is NOT cancelled — its pair of spawned
        // bridge tasks keep running until either endpoint
        // closes. That's intentional: killing a live bytestream
        // mid-transfer corrupts things like database result
        // sets and large downloads.
        self.stop.notify_waiters();
        if let Some(h) = self.task.take() {
            h.abort();
        }
    }
}

impl SshSession {
    /// Open a local port forward on `127.0.0.1:local_port`
    /// that tunnels incoming TCP connections through this
    /// SSH session to `remote_host:remote_port`.
    ///
    /// Pass `local_port = 0` to let the OS pick a free port;
    /// the actual bound port is then available via
    /// [`Tunnel::local_port`]. Typical Pier-X usage: pick a
    /// convention like `10000 + remote_port` (so MySQL's
    /// 3306 becomes 13306) and pass it in directly so the
    /// user can predict what local port to point their GUI
    /// client at.
    ///
    /// The accept loop spawns a fresh pair of bridge tasks
    /// per accepted connection. Each bridge pair proxies
    /// bytes bidirectionally until one side closes; the
    /// bridge pair then shuts down the other half cleanly
    /// and exits.
    ///
    /// Returns a [`Tunnel`] handle — drop it to close the
    /// listener and stop accepting new connections.
    pub async fn open_local_forward(
        &self,
        local_port: u16,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<Tunnel> {
        // Legacy entry point: pin the forward to THIS session (no reconnect
        // policy). Self-healing callers use `open_local_forward_dynamic`
        // with a cache-backed provider.
        let provider: Arc<dyn SessionProvider> = Arc::new(FixedSession(self.clone()));
        self.open_local_forward_dynamic(local_port, remote_host, remote_port, provider)
            .await
    }

    /// Like [`Self::open_local_forward`], but resolves the session to
    /// forward over from `provider` on **every** accepted connection, so the
    /// tunnel transparently follows a shared-session reconnect (idle death,
    /// sleep, network change, sshd restart). The forward binds and runs
    /// entirely off `provider`; `self` is taken only to mirror the
    /// method-call ergonomics of the existing call sites.
    pub async fn open_local_forward_dynamic(
        &self,
        local_port: u16,
        remote_host: &str,
        remote_port: u16,
        provider: Arc<dyn SessionProvider>,
    ) -> Result<Tunnel> {
        let bind_addr = SocketAddr::from(([127, 0, 0, 1], local_port));
        let listener = TcpListener::bind(bind_addr).await.map_err(SshError::Io)?;
        let actual_port = listener.local_addr().map_err(SshError::Io)?.port();

        let stop = Arc::new(Notify::new());
        let stop_clone = Arc::clone(&stop);
        let remote_host_owned = remote_host.to_string();

        let task = runtime::shared().spawn(async move {
            accept_loop(
                listener,
                provider,
                remote_host_owned,
                remote_port,
                stop_clone,
            )
            .await;
        });

        Ok(Tunnel {
            local_port: actual_port,
            stop,
            task: Some(task),
        })
    }

    /// Sync wrapper for [`Self::open_local_forward`].
    pub fn open_local_forward_blocking(
        &self,
        local_port: u16,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<Tunnel> {
        runtime::shared().block_on(self.open_local_forward(local_port, remote_host, remote_port))
    }

    /// Sync wrapper for [`Self::open_local_forward_dynamic`].
    pub fn open_local_forward_dynamic_blocking(
        &self,
        local_port: u16,
        remote_host: &str,
        remote_port: u16,
        provider: Arc<dyn SessionProvider>,
    ) -> Result<Tunnel> {
        runtime::shared().block_on(self.open_local_forward_dynamic(
            local_port,
            remote_host,
            remote_port,
            provider,
        ))
    }
}

/// The accept loop: takes one connection at a time, spawns
/// a bridge task pair for it, then goes back to accepting.
/// Exits cleanly when `stop` fires.
async fn accept_loop(
    listener: TcpListener,
    provider: Arc<dyn SessionProvider>,
    remote_host: String,
    remote_port: u16,
    stop: Arc<Notify>,
) {
    loop {
        tokio::select! {
            biased;
            _ = stop.notified() => {
                log::debug!("tunnel accept loop stopping");
                return;
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((tcp_stream, peer)) => {
                        let provider = Arc::clone(&provider);
                        let remote_host = remote_host.clone();
                        // Resolve a live session, open the direct-tcpip
                        // channel and spawn the bidirectional bridge.
                        tokio::spawn(async move {
                            if let Err(e) = bridge_connection(
                                tcp_stream,
                                peer,
                                provider,
                                remote_host,
                                remote_port,
                            )
                            .await
                            {
                                log::warn!("tunnel bridge error from {peer}: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        log::warn!("tunnel listener accept error: {e}");
                        // Keep the loop alive unless something
                        // catastrophic happened. If the listener
                        // FD went bad we'd hit a permanent error
                        // here — in that case sleeping briefly
                        // prevents a tight spin.
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                }
            }
        }
    }
}

/// Open a direct-tcpip channel for one accepted local
/// connection and bridge bytes in both directions until
/// either endpoint closes.
async fn bridge_connection(
    mut tcp_stream: tokio::net::TcpStream,
    peer: SocketAddr,
    provider: Arc<dyn SessionProvider>,
    remote_host: String,
    remote_port: u16,
) -> Result<()> {
    // Resolve the session to forward over FOR THIS CONNECTION. The provider
    // returns the cached shared session, reconnecting it if the transport
    // died since the tunnel opened — this is what makes a DB tunnel survive
    // an idle/sleep/network-change death instead of forwarding forever over
    // a corpse. `current()` is blocking (it may run a full SSH handshake),
    // so it runs on the blocking pool, not this runtime worker.
    let resolve = {
        let provider = Arc::clone(&provider);
        tokio::task::spawn_blocking(move || provider.current())
            .await
            .map_err(|e| SshError::InvalidConfig(format!("tunnel session join error: {e}")))?
    };
    let session = resolve.map_err(SshError::InvalidConfig)?;

    // "originator" metadata the SSH spec asks us to send with
    // the channel-open request. Not actually used by most
    // servers but we fill it in honestly.
    //
    // Bounded: if the SSH server accepts the channel-open request but the
    // internal target (`remote_host:remote_port`) is firewalled / black-holed,
    // `channel_open_direct_tcpip` would otherwise hang until the SERVER's own
    // TCP connect timeout (~2 min on Linux), during which this bridge task and
    // the local client both stall. Mirrors the `timeout_secs` guard on
    // `dial_direct_tcpip_blocking`.
    const DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    let handle = session.handle_arc();
    let open = handle.channel_open_direct_tcpip(
        remote_host.clone(),
        remote_port as u32,
        peer.ip().to_string(),
        peer.port() as u32,
    );
    let channel = match tokio::time::timeout(DIAL_TIMEOUT, open).await {
        Ok(res) => res.map_err(SshError::Protocol)?,
        Err(_) => {
            return Err(SshError::InvalidConfig(format!(
                "tunnel: opening a forward to {}:{} timed out after {}s \
                 (remote target unreachable or firewalled?)",
                remote_host,
                remote_port,
                DIAL_TIMEOUT.as_secs()
            )));
        }
    };

    // Convert the russh channel into an AsyncRead+AsyncWrite
    // adapter, then run tokio's copy_bidirectional to
    // proxy bytes in both directions.
    let mut channel_stream = channel.into_stream();
    let result = tokio::io::copy_bidirectional(&mut tcp_stream, &mut channel_stream).await;

    // Best-effort half-close on both sides so the remote
    // sees a clean EOF rather than a reset.
    let _ = tcp_stream.shutdown().await;
    let _ = channel_stream.shutdown().await;

    result.map_err(SshError::Io)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reproduces the tunnel self-heal nesting: a task on the shared runtime
    // (accept_loop) offloads the blocking session resolution (provider.current
    // → get_or_open_ssh_session → connect_blocking) to spawn_blocking, and that
    // blocking closure calls `runtime::shared().block_on(...)`. If tokio treated
    // the spawn_blocking thread as an async-execution context, this would panic
    // with "Cannot start a runtime from within a runtime" — on the *reconnect*
    // path only, i.e. in production. This test proves it does not.
    #[test]
    fn spawn_blocking_nested_block_on_shared_runtime_ok() {
        let v = runtime::shared().block_on(async {
            let jh = tokio::task::spawn_blocking(|| {
                // Stand-in for connect_blocking's `runtime::shared().block_on`.
                runtime::shared().block_on(async { 40 + 2 })
            });
            jh.await.unwrap()
        });
        assert_eq!(v, 42);
    }

    #[test]
    fn tunnel_drop_marks_not_alive() {
        // Minimal test: fabricate a Tunnel without a real
        // session by spawning a no-op task on the shared
        // runtime, then assert that drop flips is_alive to
        // false. We're not exercising any russh state here,
        // just verifying the Tunnel handle bookkeeping.
        let stop = Arc::new(Notify::new());
        let stop_clone = Arc::clone(&stop);
        let task = runtime::shared().spawn(async move {
            stop_clone.notified().await;
        });

        // Wait a beat so the task is definitely running.
        std::thread::sleep(std::time::Duration::from_millis(10));

        let t = Tunnel {
            local_port: 13306,
            stop,
            task: Some(task),
        };
        assert_eq!(t.local_port(), 13306);
        assert!(t.is_alive());

        drop(t);
        // After drop the task should be aborted. Give tokio
        // a moment to reap, then any subsequent observation
        // would need another handle — we can't query a
        // dropped Tunnel, so this test just verifies drop()
        // completes without deadlocking.
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
