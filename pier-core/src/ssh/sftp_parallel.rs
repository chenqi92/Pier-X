//! Parallel multi-file SFTP transfers.
//!
//! Splits a directory tree across N concurrent SFTP channels opened
//! on the same SSH session. Each worker pulls files off a shared
//! work queue and uploads / downloads them independently — the
//! single-channel `upload_tree_blocking` / `download_tree_blocking`
//! variants in [`super::sftp`] become the N=1 special case of what
//! lives here.
//!
//! ## Why a separate module
//!
//! The single-stream tree functions are cheap and predictable:
//! one channel, one file at a time. Parallel transfers double-down
//! on connection management — N channel handles to keep alive, a
//! work queue to coordinate them, an atomic to aggregate progress,
//! a cancellation token to short-circuit on first error. Pulling
//! that into its own file keeps `sftp.rs` focused on the one-call
//! API and makes the parallelism story easy to read on its own.
//!
//! ## Concurrency
//!
//! Per-channel uploads/downloads inherit the auto-resume behavior
//! from [`super::sftp::SftpClient::upload_from_with_progress`] /
//! `download_to_with_progress`: each worker probes the destination,
//! resumes if the destination is a strict prefix, skips on
//! same-size, truncates otherwise. So a parallel run cooperates
//! with previous interrupted runs without extra plumbing.
//!
//! Defaults to 4 channels, capped at 16. Past 4 you typically don't
//! see throughput gains because the underlying SSH transport's MAC
//! computation is the bottleneck and openssh-server's per-session
//! channel limit (default 10) will start rejecting opens.
//!
//! ## Cancellation
//!
//! Workers check the cancel token between files. A long in-flight
//! file won't abort mid-stream — that needs threading the token
//! into the chunk loop, deferred for now.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use super::error::{Result, SshError};
use super::sftp::{collect_local_tree, collect_remote_tree, SftpClient};
use super::SshSession;
use crate::ssh::runtime;

/// Default number of concurrent SFTP channels per parallel transfer.
/// 4 is the empirical sweet spot for typical broadband + SSH MAC
/// throughput; tuning higher rarely pays off.
pub const DEFAULT_PARALLEL_CONCURRENCY: usize = 4;

/// Hard ceiling. Past 16 channels openssh-server's default
/// `MaxSessions 10` starts rejecting opens, and the runtime's I/O
/// queue saturates anyway.
pub const MAX_PARALLEL_CONCURRENCY: usize = 16;

/// Tuning knobs for [`upload_tree_parallel_blocking`] /
/// [`download_tree_parallel_blocking`].
#[derive(Debug, Clone, Copy)]
pub struct ParallelOpts {
    /// Number of concurrent SFTP channels. Clamped to
    /// `[1, MAX_PARALLEL_CONCURRENCY]`.
    pub concurrency: usize,
}

impl Default for ParallelOpts {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_PARALLEL_CONCURRENCY,
        }
    }
}

impl ParallelOpts {
    fn effective(&self) -> usize {
        self.concurrency.clamp(1, MAX_PARALLEL_CONCURRENCY)
    }
}

/// Recursively upload `local_root` into `remote_root` using N
/// concurrent SFTP channels opened on `session`. Auto-resumes per
/// file (size-prefix match), skips files whose remote stat already
/// matches local size. `on_progress` reports cumulative bytes
/// across all workers.
///
/// First worker error cancels the rest; the function then returns
/// that error. If `external_cancel` is `Some` and fires, all
/// workers stop mid-chunk (the per-file functions return
/// [`SshError::Cancelled`]) and the function returns the same
/// error. The destination tree is left in its partial state — a
/// follow-up call to this function picks up where we stopped via
/// the auto-resume / skip-on-same-size machinery.
pub fn upload_tree_parallel_blocking<F>(
    session: Arc<SshSession>,
    local_root: &Path,
    remote_root: &str,
    opts: ParallelOpts,
    external_cancel: Option<CancellationToken>,
    on_progress: F,
) -> Result<u64>
where
    F: FnMut(u64, u64) + Send + 'static,
{
    let concurrency = opts.effective();

    // First pass on a sync thread: enumerate local files / dirs and
    // sum their sizes. Mirrors what the single-stream variant does
    // so the progress bar starts with an accurate total.
    let mut files: Vec<(PathBuf, String, u64)> = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    let mut total: u64 = 0;
    collect_local_tree(
        local_root,
        local_root,
        remote_root,
        &mut files,
        &mut dirs,
        &mut total,
    )?;

    let progress = Arc::new(Mutex::new(on_progress));
    {
        if let Ok(mut g) = progress.lock() {
            g(0, total);
        }
    }

    // Dirs first, files last — sort so parents always create before
    // children (collect_local_tree already returns parents-before
    // by recursion order, but defensively re-sort anyway).
    dirs.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));

    let remote_root_owned = remote_root.to_string();
    let cumulative = Arc::new(AtomicU64::new(0));
    // Internal token for first-error cancellation. We also fold in
    // the caller-provided `external_cancel` (if any) by spawning a
    // small task that mirrors its `cancel()` into our token —
    // simpler than threading two tokens through every worker.
    let cancel = CancellationToken::new();
    if let Some(ext) = external_cancel.clone() {
        let internal = cancel.clone();
        runtime::shared().spawn(async move {
            ext.cancelled().await;
            internal.cancel();
        });
    }

    runtime::shared().block_on(async move {
        // Stage 1: open N SFTP channels up front. If any open
        // fails we drop the rest before returning — no orphan
        // channels left half-initialized on the wire.
        let mut clients: Vec<SftpClient> = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            match session.open_sftp().await {
                Ok(c) => clients.push(c),
                Err(e) => {
                    drop(clients);
                    return Err(e);
                }
            }
        }

        // Stage 2: pre-create the directory tree on the remote with
        // the first worker's channel. mkdir-on-existing surfaces an
        // error from most SFTP servers; we ignore it across the
        // board — the per-file upload will surface a real
        // permission/io error if the dir is genuinely unusable.
        let setup = &clients[0];
        let _ = setup.create_dir(&remote_root_owned).await;
        for dir in &dirs {
            let _ = setup.create_dir(dir).await;
        }

        // Stage 3: build the work queue and spawn N workers.
        let queue: Arc<Mutex<VecDeque<(PathBuf, String, u64)>>> =
            Arc::new(Mutex::new(VecDeque::from(files)));
        let mut joinset: JoinSet<Result<()>> = JoinSet::new();
        for (worker_idx, client) in clients.into_iter().enumerate() {
            let queue = queue.clone();
            let progress = progress.clone();
            let cumulative = cumulative.clone();
            let cancel = cancel.clone();
            joinset.spawn(async move {
                upload_worker_loop(
                    worker_idx,
                    client,
                    queue,
                    progress,
                    cumulative,
                    cancel,
                    total,
                )
                .await
            });
        }

        // Stage 4: collect results. First error wins; we cancel
        // remaining workers and drain.
        let mut first_err: Option<SshError> = None;
        while let Some(joined) = joinset.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                        cancel.cancel();
                    }
                }
                Err(join_err) => {
                    if first_err.is_none() {
                        first_err = Some(SshError::InvalidConfig(format!(
                            "sftp parallel worker panicked: {join_err}"
                        )));
                        cancel.cancel();
                    }
                }
            }
        }

        if let Some(e) = first_err {
            return Err(e);
        }
        Ok(cumulative.load(Ordering::Relaxed))
    })
}

/// Mirror of [`upload_tree_parallel_blocking`] for downloads.
pub fn download_tree_parallel_blocking<F>(
    session: Arc<SshSession>,
    remote_root: &str,
    local_root: &Path,
    opts: ParallelOpts,
    external_cancel: Option<CancellationToken>,
    on_progress: F,
) -> Result<u64>
where
    F: FnMut(u64, u64) + Send + 'static,
{
    let concurrency = opts.effective();

    let progress = Arc::new(Mutex::new(on_progress));
    let cumulative = Arc::new(AtomicU64::new(0));
    let cancel = CancellationToken::new();
    if let Some(ext) = external_cancel.clone() {
        let internal = cancel.clone();
        runtime::shared().spawn(async move {
            ext.cancelled().await;
            internal.cancel();
        });
    }

    let local_root_owned: PathBuf = local_root.to_path_buf();
    let remote_root_owned = remote_root.to_string();

    runtime::shared().block_on(async move {
        // Open the workers up front.
        let mut clients: Vec<SftpClient> = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            match session.open_sftp().await {
                Ok(c) => clients.push(c),
                Err(e) => {
                    drop(clients);
                    return Err(e);
                }
            }
        }

        // Use the first worker to walk the remote tree.
        let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
        let mut total: u64 = 0;
        collect_remote_tree(
            &clients[0],
            &remote_root_owned,
            &local_root_owned,
            &mut files,
            &mut total,
        )
        .await?;

        if let Ok(mut g) = progress.lock() {
            g(0, total);
        }

        // Pre-create the local dir tree synchronously — local FS,
        // no SSH bandwidth involved, no need to parallelize.
        if !local_root_owned.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(&local_root_owned);
        }
        // Each file's parent dir is created by the worker before
        // opening the destination, so we don't pre-walk dirs here.

        let queue: Arc<Mutex<VecDeque<(String, PathBuf, u64)>>> =
            Arc::new(Mutex::new(VecDeque::from(files)));
        let mut joinset: JoinSet<Result<()>> = JoinSet::new();
        for (worker_idx, client) in clients.into_iter().enumerate() {
            let queue = queue.clone();
            let progress = progress.clone();
            let cumulative = cumulative.clone();
            let cancel = cancel.clone();
            joinset.spawn(async move {
                download_worker_loop(
                    worker_idx,
                    client,
                    queue,
                    progress,
                    cumulative,
                    cancel,
                    total,
                )
                .await
            });
        }

        let mut first_err: Option<SshError> = None;
        while let Some(joined) = joinset.join_next().await {
            match joined {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                        cancel.cancel();
                    }
                }
                Err(join_err) => {
                    if first_err.is_none() {
                        first_err = Some(SshError::InvalidConfig(format!(
                            "sftp parallel worker panicked: {join_err}"
                        )));
                        cancel.cancel();
                    }
                }
            }
        }

        if let Some(e) = first_err {
            return Err(e);
        }
        Ok(cumulative.load(Ordering::Relaxed))
    })
}

/// One upload worker: pop files off the queue and stream each
/// through its dedicated SFTP channel until empty or cancelled.
async fn upload_worker_loop(
    _worker_idx: usize,
    client: SftpClient,
    queue: Arc<Mutex<VecDeque<(PathBuf, String, u64)>>>,
    progress: Arc<Mutex<dyn FnMut(u64, u64) + Send>>,
    cumulative: Arc<AtomicU64>,
    cancel: CancellationToken,
    total: u64,
) -> Result<()> {
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let next = {
            let mut q = match queue.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            q.pop_front()
        };
        let Some((local_path, remote_path, _size)) = next else {
            return Ok(());
        };

        let progress = progress.clone();
        let cumulative = cumulative.clone();
        let mut last_in_file: u64 = 0;
        let result = client
            .upload_from_with_progress_cancel(
                &local_path,
                &remote_path,
                move |file_bytes, _| {
                    // Convert per-file progress to a delta and fold
                    // into the shared cumulative atomic. The
                    // tree-level callback always sees a monotonic
                    // `transferred`.
                    let delta = file_bytes.saturating_sub(last_in_file);
                    last_in_file = file_bytes;
                    if delta > 0 {
                        cumulative.fetch_add(delta, Ordering::Relaxed);
                    }
                    if let Ok(mut g) = progress.lock() {
                        let cum = cumulative.load(Ordering::Relaxed);
                        g(cum, total.max(cum));
                    }
                },
                Some(&cancel),
            )
            .await;
        if let Err(e) = result {
            return Err(e);
        }
    }
}

/// Mirror of [`upload_worker_loop`] for downloads — pops `(remote,
/// local, size)` tuples and writes each to disk.
async fn download_worker_loop(
    _worker_idx: usize,
    client: SftpClient,
    queue: Arc<Mutex<VecDeque<(String, PathBuf, u64)>>>,
    progress: Arc<Mutex<dyn FnMut(u64, u64) + Send>>,
    cumulative: Arc<AtomicU64>,
    cancel: CancellationToken,
    total: u64,
) -> Result<()> {
    loop {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let next = {
            let mut q = match queue.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            q.pop_front()
        };
        let Some((remote_path, local_path, _size)) = next else {
            return Ok(());
        };

        // Each worker creates its own destination parent dir —
        // racing on the same parent across workers is fine because
        // `create_dir_all` ignores already-exists.
        if let Some(parent) = local_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let progress = progress.clone();
        let cumulative = cumulative.clone();
        let mut last_in_file: u64 = 0;
        let result = client
            .download_to_with_progress_cancel(
                &remote_path,
                &local_path,
                move |file_bytes, _| {
                    let delta = file_bytes.saturating_sub(last_in_file);
                    last_in_file = file_bytes;
                    if delta > 0 {
                        cumulative.fetch_add(delta, Ordering::Relaxed);
                    }
                    if let Ok(mut g) = progress.lock() {
                        let cum = cumulative.load(Ordering::Relaxed);
                        g(cum, total.max(cum));
                    }
                },
                Some(&cancel),
            )
            .await;
        if let Err(e) = result {
            return Err(e);
        }
    }
}

// ── Single-file chunked parallel ────────────────────────────────

/// File size below which chunked parallel is skipped — open + close
/// + N-channel setup overhead dominates the actual transfer for
/// small files. 8 MB is a safe knee.
pub const CHUNKED_PARALLEL_MIN_BYTES: u64 = 8 * 1024 * 1024;

/// Suffix for the partial-write sibling file. Chunked transfers
/// write their byte ranges into `<final>.pierx-part`; on success
/// we rename it onto the final path. A run that died mid-flight
/// leaves the suffix behind for the next attempt to wipe.
const PARTIAL_SUFFIX: &str = ".pierx-part";

/// Upload one large file split across N concurrent SFTP channels.
/// Each worker writes a disjoint byte range to a sibling
/// `<remote>.pierx-part` file via SFTP's pwrite-style write-at-
/// offset, then we rename the part file onto the final path on
/// success. Provides a real throughput win on high-RTT links where
/// a single channel can't fill the pipe.
///
/// Falls through to single-channel
/// [`SftpClient::upload_from_with_progress_cancel`] when:
///   * file size is below [`CHUNKED_PARALLEL_MIN_BYTES`], OR
///   * `concurrency <= 1` (no parallelism requested), OR
///   * the destination already partly exists (a previous attempt
///     left bytes there) — single-channel auto-resume is the safe
///     path because chunked parallel would race and hole-fill
///     unrelated bytes.
///
/// Atomicity: the final path is only touched at the very end via
/// `remove + rename`. Cancel mid-flight or worker error leaves the
/// `.pierx-part` orphan for the next attempt to clean up — the
/// user's "real" file at `<remote>` is never in a torn state.
pub fn upload_chunked_parallel_blocking<F>(
    session: Arc<SshSession>,
    local: &Path,
    remote: &str,
    opts: ParallelOpts,
    external_cancel: Option<CancellationToken>,
    on_progress: F,
) -> Result<u64>
where
    F: FnMut(u64, u64) + Send + 'static,
{
    let concurrency = opts.effective();
    let local_owned = local.to_path_buf();
    let remote_owned = remote.to_string();

    let progress: Arc<Mutex<dyn FnMut(u64, u64) + Send>> = Arc::new(Mutex::new(on_progress));
    let cumulative = Arc::new(AtomicU64::new(0));
    let cancel = CancellationToken::new();
    if let Some(ext) = external_cancel.clone() {
        let internal = cancel.clone();
        runtime::shared().spawn(async move {
            ext.cancelled().await;
            internal.cancel();
        });
    }

    runtime::shared().block_on(async move {
        let local_meta = tokio::fs::metadata(&local_owned)
            .await
            .map_err(SshError::Io)?;
        let total = local_meta.len();

        let setup = session.open_sftp().await?;

        // Probe the destination size. Any error → treat as 0
        // (matches the single-channel auto-resume heuristic).
        let remote_size = setup.stat(&remote_owned).await.map(|e| e.size).unwrap_or(0);

        // Already complete: skip — same shortcut as single-channel.
        if remote_size == total && total > 0 {
            if let Ok(mut g) = progress.lock() {
                g(total, total);
            }
            return Ok(total);
        }

        // Below threshold OR partial state OR no-parallel-requested →
        // hand off to the single-channel auto-resume path. Reuse
        // the setup client so we don't pay a second channel-open
        // RTT on the small-file fast path.
        if total < CHUNKED_PARALLEL_MIN_BYTES || concurrency <= 1 || remote_size > 0 {
            let progress = progress.clone();
            return setup
                .upload_from_with_progress_cancel(
                    &local_owned,
                    &remote_owned,
                    move |b, t| {
                        if let Ok(mut g) = progress.lock() {
                            g(b, t);
                        }
                    },
                    Some(&cancel),
                )
                .await;
        }

        // Chunked path: scaffold the .pierx-part file at size 0 so
        // workers' WRITE | CREATE opens land on an existing inode
        // they can pwrite into. Best-effort wipe of any leftover
        // .pierx-part from a previous failed attempt before
        // creating a fresh one.
        let part_path = format!("{remote_owned}{PARTIAL_SUFFIX}");
        let _ = setup.remove_file(&part_path).await;
        setup
            .create_file(&part_path)
            .await
            .map_err(|e| SshError::InvalidConfig(format!("create {part_path}: {e}")))?;

        if let Ok(mut g) = progress.lock() {
            g(0, total);
        }

        // Compute byte ranges for N workers — last range absorbs
        // the remainder so we don't drift on non-divisible sizes.
        let chunk_size = total.div_ceil(concurrency as u64);
        let mut ranges: Vec<(u64, u64)> = Vec::with_capacity(concurrency);
        for i in 0..concurrency {
            let start = (i as u64) * chunk_size;
            if start >= total {
                break;
            }
            let end = (start + chunk_size).min(total);
            ranges.push((start, end));
        }

        // Open one channel per range. setup channel stays for the
        // final rename + cleanup.
        let mut clients: Vec<SftpClient> = Vec::with_capacity(ranges.len());
        for _ in 0..ranges.len() {
            match session.open_sftp().await {
                Ok(c) => clients.push(c),
                Err(e) => {
                    drop(clients);
                    let _ = setup.remove_file(&part_path).await;
                    return Err(e);
                }
            }
        }

        let mut joinset: JoinSet<Result<u64>> = JoinSet::new();
        for (client, (start, end)) in clients.into_iter().zip(ranges.into_iter()) {
            let local_path = local_owned.clone();
            let part_path_for_w = part_path.clone();
            let progress = progress.clone();
            let cumulative = cumulative.clone();
            let cancel = cancel.clone();
            joinset.spawn(async move {
                let mut last_reported: u64 = 0;
                client
                    .upload_range_with_progress_cancel(
                        &local_path,
                        &part_path_for_w,
                        start,
                        end,
                        move |range_bytes| {
                            let delta = range_bytes.saturating_sub(last_reported);
                            last_reported = range_bytes;
                            if delta > 0 {
                                cumulative.fetch_add(delta, Ordering::Relaxed);
                            }
                            if let Ok(mut g) = progress.lock() {
                                let cum = cumulative.load(Ordering::Relaxed);
                                g(cum, total.max(cum));
                            }
                        },
                        Some(&cancel),
                    )
                    .await
            });
        }

        let mut first_err: Option<SshError> = None;
        while let Some(joined) = joinset.join_next().await {
            match joined {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                        cancel.cancel();
                    }
                }
                Err(je) => {
                    if first_err.is_none() {
                        first_err = Some(SshError::InvalidConfig(format!(
                            "chunked upload worker panic: {je}"
                        )));
                        cancel.cancel();
                    }
                }
            }
        }

        if let Some(e) = first_err {
            // Leave the .pierx-part where it is — the next attempt
            // wipes it before re-creating. Avoids racy double-delete
            // when the user immediately retries.
            return Err(e);
        }

        // All workers OK → move the part file into the final
        // location atomically. Best-effort remove the destination
        // first so the rename never collides with a stale older
        // file (some servers reject overwrite-rename).
        let _ = setup.remove_file(&remote_owned).await;
        setup
            .rename(&part_path, &remote_owned)
            .await
            .map_err(|e| SshError::InvalidConfig(format!("rename part->final: {e}")))?;
        Ok(total)
    })
}

/// Mirror of [`upload_chunked_parallel_blocking`] for downloads.
/// Workers write disjoint ranges into a local `<local>.pierx-part`
/// file pre-allocated to `total` bytes; on success we rename onto
/// the final path. Same fall-through rules: tiny files, no-
/// parallelism, or pre-existing partial state hand off to the
/// single-channel auto-resume path.
pub fn download_chunked_parallel_blocking<F>(
    session: Arc<SshSession>,
    remote: &str,
    local: &Path,
    opts: ParallelOpts,
    external_cancel: Option<CancellationToken>,
    on_progress: F,
) -> Result<u64>
where
    F: FnMut(u64, u64) + Send + 'static,
{
    let concurrency = opts.effective();
    let local_owned = local.to_path_buf();
    let remote_owned = remote.to_string();

    let progress: Arc<Mutex<dyn FnMut(u64, u64) + Send>> = Arc::new(Mutex::new(on_progress));
    let cumulative = Arc::new(AtomicU64::new(0));
    let cancel = CancellationToken::new();
    if let Some(ext) = external_cancel.clone() {
        let internal = cancel.clone();
        runtime::shared().spawn(async move {
            ext.cancelled().await;
            internal.cancel();
        });
    }

    runtime::shared().block_on(async move {
        let setup = session.open_sftp().await?;
        let total = setup
            .stat(&remote_owned)
            .await
            .map(|e| e.size)
            .map_err(|e| SshError::InvalidConfig(format!("stat {remote_owned}: {e}")))?;

        let local_size = match tokio::fs::metadata(&local_owned).await {
            Ok(m) => m.len(),
            Err(_) => 0,
        };

        if local_size == total && total > 0 {
            if let Ok(mut g) = progress.lock() {
                g(total, total);
            }
            return Ok(total);
        }

        if total < CHUNKED_PARALLEL_MIN_BYTES || concurrency <= 1 || local_size > 0 {
            let progress = progress.clone();
            return setup
                .download_to_with_progress_cancel(
                    &remote_owned,
                    &local_owned,
                    move |b, t| {
                        if let Ok(mut g) = progress.lock() {
                            g(b, t);
                        }
                    },
                    Some(&cancel),
                )
                .await;
        }

        // Pre-create a sized .pierx-part next to the destination
        // so workers can pwrite into disjoint regions.
        let mut part_path = local_owned.clone();
        let new_name = format!(
            "{}{}",
            local_owned
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("download"),
            PARTIAL_SUFFIX
        );
        part_path.set_file_name(new_name);

        if let Some(parent) = part_path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
        }
        let _ = tokio::fs::remove_file(&part_path).await;
        {
            let f = tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&part_path)
                .await
                .map_err(SshError::Io)?;
            f.set_len(total).await.map_err(SshError::Io)?;
        }

        if let Ok(mut g) = progress.lock() {
            g(0, total);
        }

        let chunk_size = total.div_ceil(concurrency as u64);
        let mut ranges: Vec<(u64, u64)> = Vec::with_capacity(concurrency);
        for i in 0..concurrency {
            let start = (i as u64) * chunk_size;
            if start >= total {
                break;
            }
            let end = (start + chunk_size).min(total);
            ranges.push((start, end));
        }

        let mut clients: Vec<SftpClient> = Vec::with_capacity(ranges.len());
        for _ in 0..ranges.len() {
            match session.open_sftp().await {
                Ok(c) => clients.push(c),
                Err(e) => {
                    drop(clients);
                    let _ = tokio::fs::remove_file(&part_path).await;
                    return Err(e);
                }
            }
        }

        let mut joinset: JoinSet<Result<u64>> = JoinSet::new();
        for (client, (start, end)) in clients.into_iter().zip(ranges.into_iter()) {
            let part_path_for_w = part_path.clone();
            let remote_path = remote_owned.clone();
            let progress = progress.clone();
            let cumulative = cumulative.clone();
            let cancel = cancel.clone();
            joinset.spawn(async move {
                let mut last_reported: u64 = 0;
                client
                    .download_range_with_progress_cancel(
                        &remote_path,
                        &part_path_for_w,
                        start,
                        end,
                        move |range_bytes| {
                            let delta = range_bytes.saturating_sub(last_reported);
                            last_reported = range_bytes;
                            if delta > 0 {
                                cumulative.fetch_add(delta, Ordering::Relaxed);
                            }
                            if let Ok(mut g) = progress.lock() {
                                let cum = cumulative.load(Ordering::Relaxed);
                                g(cum, total.max(cum));
                            }
                        },
                        Some(&cancel),
                    )
                    .await
            });
        }

        let mut first_err: Option<SshError> = None;
        while let Some(joined) = joinset.join_next().await {
            match joined {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                        cancel.cancel();
                    }
                }
                Err(je) => {
                    if first_err.is_none() {
                        first_err = Some(SshError::InvalidConfig(format!(
                            "chunked download worker panic: {je}"
                        )));
                        cancel.cancel();
                    }
                }
            }
        }

        if let Some(e) = first_err {
            return Err(e);
        }

        // Atomic rename onto the final path. We pre-created the
        // .pierx-part next to `local` so they're always on the same
        // filesystem.
        let _ = tokio::fs::remove_file(&local_owned).await;
        tokio::fs::rename(&part_path, &local_owned)
            .await
            .map_err(SshError::Io)?;
        Ok(total)
    })
}
