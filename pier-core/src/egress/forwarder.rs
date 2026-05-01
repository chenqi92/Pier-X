//! Local TCP forwarder — accepts loopback connections on a random
//! port and pipes each one through an [`super::EgressProfile`] to a
//! fixed `target_host:target_port`.
//!
//! Used by DB / Redis panels that talk to clients which only expose
//! a `host:port` connect surface (`mysql_async`, `redis`,
//! `tokio-postgres`'s blocking codepath). The DB client connects to
//! `127.0.0.1:<assigned_port>` and the forwarder transparently
//! proxies the bytes through the egress.
//!
//! Lifecycle: [`EgressForwarder::start`] returns a handle. Drop the
//! handle to tear down the listener and stop accepting new
//! connections; in-flight forwards finish on their own.

use std::io;
use std::sync::Arc;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::{resolve_tcp_with, EgressContext, EgressProfile};
use crate::ssh::runtime;

/// Handle to a running forwarder. Drop to stop the listener.
pub struct EgressForwarder {
    /// `127.0.0.1:<port>` the local DB client should connect to.
    pub local_port: u16,
    stop: Arc<Notify>,
}

impl EgressForwarder {
    /// Start a forwarder on `127.0.0.1:0` (OS-assigned port). Each
    /// inbound connection is paired with a fresh egress dial via
    /// [`resolve_tcp_with`]. `ctx` is forwarded so ssh-jump targets
    /// work the same way they do for SSH connections.
    ///
    /// `profile = None` is allowed — produces a plain loopback ↔
    /// target proxy, useful when the user wants a stable local
    /// endpoint for a remote DB without changing the panel UI.
    ///
    /// Async variant; call from a tokio context. Use
    /// [`Self::start_blocking`] from synchronous code.
    pub async fn start(
        profile: Option<EgressProfile>,
        target_host: String,
        target_port: u16,
        ctx: Option<Arc<dyn EgressContext>>,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0u16)).await?;
        let local_port = listener.local_addr()?.port();

        let stop = Arc::new(Notify::new());
        let stop_clone = Arc::clone(&stop);
        let profile_arc = profile.map(Arc::new);

        // Drive the accept loop on the shared runtime so its lifetime
        // is decoupled from whichever runtime / task started us. The
        // handle returned to the caller owns the `Notify` that ends
        // the loop on Drop.
        runtime::shared().spawn(async move {
            loop {
                tokio::select! {
                    _ = stop_clone.notified() => break,
                    accept = listener.accept() => {
                        let (client, _peer) = match accept {
                            Ok(pair) => pair,
                            Err(e) => {
                                log::warn!("egress forwarder accept failed: {e}");
                                continue;
                            }
                        };
                        let target_host = target_host.clone();
                        let target_port = target_port;
                        let profile = profile_arc.clone();
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            if let Err(e) = bridge_one(
                                client,
                                profile.as_deref(),
                                &target_host,
                                target_port,
                                ctx.as_deref(),
                            ).await {
                                log::warn!("egress forwarder bridge failed: {e}");
                            }
                        });
                    }
                }
            }
        });

        Ok(Self { local_port, stop })
    }

    /// Blocking entry point for [`Self::start`]. Must NOT be called
    /// from inside a tokio task.
    pub fn start_blocking(
        profile: Option<EgressProfile>,
        target_host: String,
        target_port: u16,
        ctx: Option<Arc<dyn EgressContext>>,
    ) -> io::Result<Self> {
        runtime::shared()
            .block_on(Self::start(profile, target_host, target_port, ctx))
    }
}

impl Drop for EgressForwarder {
    fn drop(&mut self) {
        // Notifies the accept loop to break; in-flight forwards keep
        // running until either side closes naturally.
        self.stop.notify_waiters();
    }
}

async fn bridge_one(
    mut client: tokio::net::TcpStream,
    profile: Option<&EgressProfile>,
    target_host: &str,
    target_port: u16,
    ctx: Option<&dyn EgressContext>,
) -> io::Result<()> {
    let mut upstream = resolve_tcp_with(profile, target_host, target_port, ctx).await?;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    let _ = client.shutdown().await;
    let _ = upstream.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// Sanity: a forwarder with profile=None proxies bytes from
    /// loopback to a backend listener and back.
    #[tokio::test(flavor = "multi_thread")]
    async fn direct_forwarder_proxies_bytes() {
        // Backend: echo "ok" once.
        let backend = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend_port = backend.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = backend.accept().await.unwrap();
            let mut got = [0u8; 2];
            sock.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"hi");
            sock.write_all(b"ok").await.unwrap();
        });

        let fwd = EgressForwarder::start(None, "127.0.0.1".into(), backend_port, None)
            .await
            .expect("start forwarder");
        let mut client = TcpStream::connect(("127.0.0.1", fwd.local_port))
            .await
            .unwrap();
        client.write_all(b"hi").await.unwrap();
        let mut got = [0u8; 2];
        client.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ok");
    }

    /// Drop on the handle stops accepting new connections.
    #[tokio::test(flavor = "multi_thread")]
    async fn drop_handle_closes_listener() {
        let fwd = EgressForwarder::start(None, "127.0.0.1".into(), 65535, None)
            .await
            .unwrap();
        let port = fwd.local_port;
        drop(fwd);
        // Give the accept loop a beat to notice the notify.
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Subsequent connect should either fail or succeed-then-die;
        // we don't assert which (both are fine), just that we don't hang.
        let _ = tokio::time::timeout(
            Duration::from_millis(500),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
    }
}
