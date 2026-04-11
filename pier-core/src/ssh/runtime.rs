//! Process-wide tokio runtime shared by every SSH session.
//!
//! russh is async. The `Pty` trait that [`super::channel::SshChannelPty`]
//! implements is sync and non-blocking. The mismatch is bridged
//! inside [`super::channel`]: a dedicated tokio task drives the
//! russh side of each channel, and sync/async mpsc queues move
//! bytes across the boundary.
//!
//! That task, and the russh I/O it does, runs on **one tokio
//! runtime shared by every SSH session in the process**. A fresh
//! runtime per session would spawn a fresh multi-thread worker
//! pool (several OS threads) each time, which is wasteful at any
//! realistic pier-x session count.
//!
//! The runtime is created lazily on first use via a `OnceLock`
//! and lives for the rest of the process. It never gets shut down
//! explicitly — tokio runtimes released on program exit don't
//! need explicit shutdown, and the alternative (a global
//! `Arc<Runtime>` with refcount-based teardown) complicates
//! error paths for no real benefit.
//!
//! ## Sizing
//!
//! Two worker threads by default. SSH is I/O-bound — a single
//! thread would work for most users, but two gives us headroom
//! for parallel `tcpip-forward` tunnels + channels without
//! tuning. When pier-core grows more async subsystems (git,
//! database clients) they can reuse this same runtime rather
//! than each spawning their own pool.

use std::sync::OnceLock;
use tokio::runtime::{Builder, Runtime};

/// Returns a reference to the process-wide tokio runtime used by
/// every async subsystem in pier-core (today: just SSH).
///
/// First call builds the runtime; subsequent calls are a simple
/// atomic load. Panics only if the first-build path fails — which
/// in practice only happens on systems so starved for threads
/// that spawning an OS thread pool is impossible, in which case
/// SSH wouldn't work anyway.
pub fn shared() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pier-async")
            .enable_io()
            .enable_time()
            .build()
            .expect("failed to build pier-core shared async runtime")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn shared_runtime_is_singleton() {
        let a = shared() as *const Runtime;
        let b = shared() as *const Runtime;
        assert_eq!(a, b, "shared() must always return the same runtime");
    }

    #[test]
    fn shared_runtime_can_run_tasks() {
        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        shared().block_on(async move {
            tokio::task::yield_now().await;
            c.fetch_add(1, Ordering::Relaxed);
        });
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }
}
