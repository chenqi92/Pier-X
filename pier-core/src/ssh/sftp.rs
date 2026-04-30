//! SFTP client — file-oriented API on top of a live
//! [`super::SshSession`].
//!
//! ## Shape
//!
//! [`SftpClient`] wraps a `russh_sftp::client::SftpSession`
//! opened on a new SSH channel, and exposes the operations
//! pier-x's shell cares about:
//!
//!   * `list_dir(path)` → `Vec<RemoteFileEntry>` sorted with
//!     directories first, then case-insensitive by name.
//!   * `read_file(path)` / `write_file(path, bytes)` for
//!     whole-file transfers under the default russh-sftp size
//!     ceiling (we log a warning above 128 MB; chunked
//!     transfers land in M3d+).
//!   * `create_dir`, `remove_file`, `remove_dir`, `rename`.
//!   * `canonicalize(path)` for the "pwd" button on the
//!     file-browser panel.
//!   * `stat(path)` → `RemoteFileEntry` for a single node.
//!
//! All async methods have a `_blocking` counterpart that
//! enters the shared runtime and `block_on`s, matching the
//! pattern `SshSession` uses. The blocking variants are what
//! the command layer calls; async tasks should stay on the
//! direct form.
//!
//! ## Not yet
//!
//! * Chunked upload/download with progress callbacks. M3d+
//!   adds a streaming `open_read` / `open_write` pair plus a
//!   progress channel the UI binds to.
//! * Recursive operations (`rm -rf`, `mkdir -p`). The shell
//!   composes these from the single-step primitives today;
//!   we'll move them into this module when recursion becomes
//!   annoying to orchestrate above this layer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh_sftp::client::SftpSession;
use russh_sftp::protocol::FileAttributes;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::error::{Result, SshError};
use super::runtime;

/// One remote filesystem entry. Serializable so the shell can
/// move it through command payloads without re-inventing the
/// schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileEntry {
    /// Leaf name (no directory component).
    pub name: String,
    /// Full remote path, including parent.
    pub path: String,
    /// True if this is a directory. Symlinks that point at
    /// directories are also reported as `is_dir = true` to
    /// match what every file browser expects.
    pub is_dir: bool,
    /// True if this is a symbolic link. `is_dir` and `is_link`
    /// can both be true.
    pub is_link: bool,
    /// Size in bytes. 0 for directories and unknown sizes.
    pub size: u64,
    /// Last modified time in seconds since the Unix epoch, if
    /// the server provided one.
    pub modified: Option<u64>,
    /// POSIX permission bits, if the server provided them.
    pub permissions: Option<u32>,
    /// Owner name from the server's longname response. SFTPv3
    /// servers may omit this; falls back to a stringified `uid`
    /// when only the numeric id is known. `None` when neither is
    /// available (most often for read-only servers running on
    /// hosts where the SSH user can't enumerate `/etc/passwd`).
    pub owner: Option<String>,
    /// Group name (or stringified `gid`). Same fallback rules as
    /// `owner`.
    pub group: Option<String>,
}

/// SFTP session handle. Cheap to clone — the underlying
/// russh-sftp session is reference-counted.
///
/// Obtain one via [`super::SshSession::open_sftp`] (or its
/// `_blocking` cousin) once the SSH handshake has completed.
#[derive(Clone)]
pub struct SftpClient {
    inner: Arc<SftpSession>,
}

impl std::fmt::Debug for SftpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SftpClient")
            .field("refcount", &Arc::strong_count(&self.inner))
            .finish()
    }
}

impl SftpClient {
    /// Internal constructor used by
    /// [`super::SshSession::open_sftp`]. Not exposed publicly
    /// because building an `SftpSession` from scratch requires
    /// the channel stream type, which is a russh implementation
    /// detail we prefer to keep hidden.
    pub(super) fn new(session: SftpSession) -> Self {
        Self {
            inner: Arc::new(session),
        }
    }

    // ── Directory listing ─────────────────────────────────

    /// List the contents of `path`, returning entries sorted
    /// directories-first, then alphabetical by name
    /// (case-insensitive). `.` and `..` are filtered out.
    pub async fn list_dir(&self, path: &str) -> Result<Vec<RemoteFileEntry>> {
        let reader = self
            .inner
            .read_dir(path.to_string())
            .await
            .map_err(sftp_error)?;

        let mut out = Vec::new();
        for entry in reader {
            let name = entry.file_name();
            if name == "." || name == ".." {
                continue;
            }

            // Build the absolute remote path. Normalize the
            // parent to have exactly one trailing slash so the
            // join is always `parent/name` regardless of input.
            let full_path = if path == "/" {
                format!("/{name}")
            } else {
                format!("{}/{}", path.trim_end_matches('/'), name)
            };

            let file_type = entry.file_type();
            let metadata = entry.metadata();
            let owner = pick_owner(metadata.user.as_deref(), metadata.uid);
            let group = pick_owner(metadata.group.as_deref(), metadata.gid);
            out.push(RemoteFileEntry {
                name,
                path: full_path,
                is_dir: file_type.is_dir(),
                is_link: file_type.is_symlink(),
                size: metadata.size.unwrap_or(0),
                modified: metadata.mtime.map(|v| v as u64),
                permissions: metadata.permissions,
                owner,
                group,
            });
        }

        // Directories first, then case-insensitive name order.
        out.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(out)
    }

    /// Synchronous wrapper for [`Self::list_dir`].
    pub fn list_dir_blocking(&self, path: &str) -> Result<Vec<RemoteFileEntry>> {
        runtime::shared().block_on(self.list_dir(path))
    }

    /// Look up a single node. Returns a [`RemoteFileEntry`]
    /// with `name` set to the leaf of `path`.
    pub async fn stat(&self, path: &str) -> Result<RemoteFileEntry> {
        let metadata = self
            .inner
            .metadata(path.to_string())
            .await
            .map_err(sftp_error)?;

        let name = Path::new(path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());

        // russh-sftp's `Metadata::file_type()` returns a
        // `FileType` wrapper similar to what ReadDir uses, so
        // we branch on that. A missing `file_type()` case
        // (very old SFTP servers that don't return type info)
        // is treated as "regular file".
        let file_type = metadata.file_type();
        let owner = pick_owner(metadata.user.as_deref(), metadata.uid);
        let group = pick_owner(metadata.group.as_deref(), metadata.gid);
        Ok(RemoteFileEntry {
            name,
            path: path.to_string(),
            is_dir: file_type.is_dir(),
            is_link: file_type.is_symlink(),
            size: metadata.size.unwrap_or(0),
            modified: metadata.mtime.map(|v| v as u64),
            permissions: metadata.permissions,
            owner,
            group,
        })
    }

    /// Synchronous wrapper for [`Self::stat`].
    pub fn stat_blocking(&self, path: &str) -> Result<RemoteFileEntry> {
        runtime::shared().block_on(self.stat(path))
    }

    // ── Whole-file transfers ──────────────────────────────

    /// Read the entire remote file into memory.
    ///
    /// Intended for configuration files, logs under ~1 MB, and
    /// similar. Larger files emit a `log::warn!` line and are
    /// still served, but M3d+ will add a streaming variant
    /// with progress callbacks for multi-MB transfers.
    pub async fn read_file(&self, path: &str) -> Result<Vec<u8>> {
        let data = self
            .inner
            .read(path.to_string())
            .await
            .map_err(sftp_error)?;
        if data.len() > 128 * 1024 * 1024 {
            log::warn!(
                "read_file({path}) loaded {} MB into memory — chunked read lands with M3d+",
                data.len() / 1_000_000,
            );
        }
        Ok(data)
    }

    /// Sync wrapper for [`Self::read_file`].
    pub fn read_file_blocking(&self, path: &str) -> Result<Vec<u8>> {
        runtime::shared().block_on(self.read_file(path))
    }

    /// Write `data` to `path`, overwriting any existing file.
    pub async fn write_file(&self, path: &str, data: &[u8]) -> Result<()> {
        if data.len() > 128 * 1024 * 1024 {
            log::warn!(
                "write_file({path}) writing {} MB through whole-file API",
                data.len() / 1_000_000,
            );
        }
        self.inner
            .write(path.to_string(), data)
            .await
            .map_err(sftp_error)?;
        Ok(())
    }

    /// Sync wrapper for [`Self::write_file`].
    pub fn write_file_blocking(&self, path: &str, data: &[u8]) -> Result<()> {
        runtime::shared().block_on(self.write_file(path, data))
    }

    /// Download `remote` to the local filesystem at `local`.
    /// Convenience wrapper around [`Self::read_file`] plus
    /// [`tokio::fs::write`].
    pub async fn download_to(&self, remote: &str, local: &Path) -> Result<()> {
        let data = self.read_file(remote).await?;
        tokio::fs::write(local, data).await.map_err(SshError::Io)?;
        log::info!("downloaded {remote} -> {local}", local = local.display());
        Ok(())
    }

    /// Sync wrapper for [`Self::download_to`].
    pub fn download_to_blocking(&self, remote: &str, local: &Path) -> Result<()> {
        runtime::shared().block_on(self.download_to(remote, local))
    }

    /// Upload local file at `local` to remote path `remote`.
    /// Convenience wrapper around [`tokio::fs::read`] plus
    /// [`Self::write_file`].
    pub async fn upload_from(&self, local: &Path, remote: &str) -> Result<()> {
        let data = tokio::fs::read(local).await.map_err(SshError::Io)?;
        self.write_file(remote, &data).await?;
        log::info!("uploaded {local} -> {remote}", local = local.display());
        Ok(())
    }

    /// Sync wrapper for [`Self::upload_from`].
    pub fn upload_from_blocking(&self, local: &Path, remote: &str) -> Result<()> {
        runtime::shared().block_on(self.upload_from(local, remote))
    }

    /// Thin wrapper over [`Self::download_to_with_progress_cancel`]
    /// for callers that don't need mid-transfer cancellation. See
    /// the cancellable variant for the full semantics.
    pub async fn download_to_with_progress<F>(
        &self,
        remote: &str,
        local: &Path,
        on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        self.download_to_with_progress_cancel(remote, local, on_progress, None)
            .await
    }

    /// Chunked download with a byte-level progress callback and
    /// optional mid-transfer cancellation. The callback fires once
    /// at the start and after every chunk with `(bytes_written,
    /// total)`. Reads 64 KiB at a time — russh-sftp's
    /// `SSH_FXP_READ` payload limit is 255 KiB so we stay well
    /// under, and 64 KiB gives progress events fine-grained enough
    /// for a smooth UI bar.
    ///
    /// Auto-resume: if a local file already exists at `local` and
    /// is a strict prefix of the remote (i.e. `0 < local_size <
    /// remote_size`), we open the existing local file in append
    /// mode and stream only the remaining bytes from the remote —
    /// the rsync `--append` model. This makes a re-issued download
    /// after a network blip cheap. If the local file is the same
    /// size as remote we treat it as "already complete" and report
    /// total without transferring; if it's larger we truncate and
    /// start over (the safe choice — local got fatter than source).
    ///
    /// Cancellation: if `cancel` is `Some` and fires between
    /// chunks, the function returns [`SshError::Cancelled`]
    /// immediately without finishing the file. The destination
    /// file is left in its partial state — auto-resume on retry
    /// picks up where we stopped.
    pub async fn download_to_with_progress_cancel<F>(
        &self,
        remote: &str,
        local: &Path,
        mut on_progress: F,
        cancel: Option<&CancellationToken>,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        use std::io::SeekFrom;
        use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

        let mut remote_file = self
            .inner
            .open(remote.to_string())
            .await
            .map_err(sftp_error)?;
        let total = remote_file
            .metadata()
            .await
            .map_err(sftp_error)?
            .size
            .unwrap_or(0);

        // Probe local for the resume offset. Any error (file
        // missing, perm) collapses to "no resume" — we'll truncate.
        let local_size = match tokio::fs::metadata(local).await {
            Ok(m) => m.len(),
            Err(_) => 0,
        };
        let resume_offset = if local_size > 0 && local_size < total {
            local_size
        } else {
            0
        };

        // Already complete — caller asked us to download a file we
        // already have at the right size. Skip the transfer entirely
        // and report a complete progress event so the UI bar lands
        // at 100% without flickering through 0.
        if local_size == total && total > 0 {
            on_progress(total, total);
            let _ = remote_file.shutdown().await;
            return Ok(total);
        }

        on_progress(resume_offset, total);

        // Make sure the parent directory exists so this mirrors
        // tokio::fs::write's "create missing path" behaviour for
        // drop-in callers.
        if let Some(parent) = local.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
        }
        let mut local_file = if resume_offset > 0 {
            // Open existing file for write WITHOUT truncate so we
            // can seek into it and append the tail bytes.
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(local)
                .await
                .map_err(SshError::Io)?
        } else {
            tokio::fs::File::create(local).await.map_err(SshError::Io)?
        };
        if resume_offset > 0 {
            remote_file
                .seek(SeekFrom::Start(resume_offset))
                .await
                .map_err(SshError::Io)?;
            local_file
                .seek(SeekFrom::Start(resume_offset))
                .await
                .map_err(SshError::Io)?;
        }
        let mut buf = vec![0u8; 64 * 1024];
        let mut transferred: u64 = resume_offset;
        loop {
            // Cancellation is checked between chunks — fine-grained
            // enough for a 64 KiB resolution while keeping the chunk
            // loop branch-free in the common case.
            if let Some(token) = cancel {
                if token.is_cancelled() {
                    let _ = local_file.flush().await;
                    let _ = remote_file.shutdown().await;
                    return Err(SshError::Cancelled);
                }
            }
            let n = remote_file.read(&mut buf).await.map_err(SshError::Io)?;
            if n == 0 {
                break;
            }
            local_file
                .write_all(&buf[..n])
                .await
                .map_err(SshError::Io)?;
            transferred += n as u64;
            on_progress(transferred, total.max(transferred));
        }
        local_file.flush().await.map_err(SshError::Io)?;
        // Explicit shutdown closes the remote file handle; ignore
        // errors (some servers send a harmless "already closed").
        let _ = remote_file.shutdown().await;
        if resume_offset > 0 {
            log::info!(
                "downloaded {remote} -> {local} (resumed at {resume_offset}/{total})",
                local = local.display(),
            );
        } else {
            log::info!(
                "downloaded {remote} -> {local} ({transferred} bytes)",
                local = local.display(),
            );
        }
        Ok(transferred)
    }

    /// Sync wrapper for [`Self::download_to_with_progress`].
    pub fn download_to_with_progress_blocking<F>(
        &self,
        remote: &str,
        local: &Path,
        on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        self.download_to_with_progress_cancel_blocking(remote, local, on_progress, None)
    }

    /// Sync wrapper for [`Self::download_to_with_progress_cancel`].
    pub fn download_to_with_progress_cancel_blocking<F>(
        &self,
        remote: &str,
        local: &Path,
        on_progress: F,
        cancel: Option<&CancellationToken>,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        runtime::shared().block_on(self.download_to_with_progress_cancel(
            remote,
            local,
            on_progress,
            cancel,
        ))
    }

    /// Thin wrapper over [`Self::upload_from_with_progress_cancel`]
    /// for callers that don't need mid-transfer cancellation.
    pub async fn upload_from_with_progress<F>(
        &self,
        local: &Path,
        remote: &str,
        on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        self.upload_from_with_progress_cancel(local, remote, on_progress, None)
            .await
    }

    /// Chunked upload with a byte-level progress callback and
    /// optional mid-transfer cancellation. Same auto-resume
    /// semantics as [`Self::download_to_with_progress_cancel`]
    /// but in the opposite direction: the local file's size is
    /// the `total`, and we resume when the remote already holds
    /// a strict prefix of the local content. The callback fires
    /// after each 64 KiB chunk is written; cancel is checked
    /// between chunks.
    pub async fn upload_from_with_progress_cancel<F>(
        &self,
        local: &Path,
        remote: &str,
        mut on_progress: F,
        cancel: Option<&CancellationToken>,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        use russh_sftp::protocol::OpenFlags;
        use std::io::SeekFrom;
        use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

        let meta = tokio::fs::metadata(local).await.map_err(SshError::Io)?;
        let total = meta.len();

        // Probe the destination so we can decide whether to resume
        // or truncate-overwrite. Any error (NoSuchFile, perm) →
        // remote_size=0 which falls through to the truncate path
        // exactly like the pre-resume implementation.
        let remote_size = match self.inner.metadata(remote.to_string()).await {
            Ok(m) => m.size.unwrap_or(0),
            Err(_) => 0,
        };
        let resume_offset = if remote_size > 0 && remote_size < total {
            remote_size
        } else {
            0
        };

        // Already complete — same behavior as the download path:
        // skip the transfer and report a single completion event.
        if remote_size == total && total > 0 {
            on_progress(total, total);
            return Ok(total);
        }

        on_progress(resume_offset, total);

        let mut local_file = tokio::fs::File::open(local).await.map_err(SshError::Io)?;
        let mut remote_file = if resume_offset > 0 {
            // WRITE | CREATE without TRUNCATE keeps the existing
            // bytes intact so seek + write tail extends the file.
            self.inner
                .open_with_flags(
                    remote.to_string(),
                    OpenFlags::WRITE | OpenFlags::CREATE,
                )
                .await
                .map_err(sftp_error)?
        } else {
            // create() = WRITE | CREATE | TRUNCATE — start fresh.
            self.inner
                .create(remote.to_string())
                .await
                .map_err(sftp_error)?
        };

        if resume_offset > 0 {
            local_file
                .seek(SeekFrom::Start(resume_offset))
                .await
                .map_err(SshError::Io)?;
            remote_file
                .seek(SeekFrom::Start(resume_offset))
                .await
                .map_err(SshError::Io)?;
        }

        let mut buf = vec![0u8; 64 * 1024];
        let mut transferred: u64 = resume_offset;
        loop {
            // Cancel between chunks. On hit we still flush+shutdown
            // the remote handle so the bytes already written aren't
            // dangling in russh's send buffer — important for resume
            // on retry which reads the remote file's current size.
            if let Some(token) = cancel {
                if token.is_cancelled() {
                    let _ = remote_file.flush().await;
                    let _ = remote_file.shutdown().await;
                    return Err(SshError::Cancelled);
                }
            }
            let n = local_file.read(&mut buf).await.map_err(SshError::Io)?;
            if n == 0 {
                break;
            }
            remote_file
                .write_all(&buf[..n])
                .await
                .map_err(SshError::Io)?;
            transferred += n as u64;
            on_progress(transferred, total.max(transferred));
        }
        remote_file.flush().await.map_err(SshError::Io)?;
        let _ = remote_file.shutdown().await;
        if resume_offset > 0 {
            log::info!(
                "uploaded {local} -> {remote} (resumed at {resume_offset}/{total})",
                local = local.display(),
            );
        } else {
            log::info!(
                "uploaded {local} -> {remote} ({transferred} bytes)",
                local = local.display(),
            );
        }
        Ok(transferred)
    }

    /// Sync wrapper for [`Self::upload_from_with_progress`].
    pub fn upload_from_with_progress_blocking<F>(
        &self,
        local: &Path,
        remote: &str,
        on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        self.upload_from_with_progress_cancel_blocking(local, remote, on_progress, None)
    }

    /// Sync wrapper for [`Self::upload_from_with_progress_cancel`].
    pub fn upload_from_with_progress_cancel_blocking<F>(
        &self,
        local: &Path,
        remote: &str,
        on_progress: F,
        cancel: Option<&CancellationToken>,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        runtime::shared().block_on(self.upload_from_with_progress_cancel(
            local,
            remote,
            on_progress,
            cancel,
        ))
    }

    /// Write bytes from `local[start..end]` to `remote` at offset
    /// `start`. The remote file must already exist (or be openable
    /// with `WRITE | CREATE` — this function never truncates).
    /// Used by the chunked-parallel single-file uploader: each
    /// worker handles one disjoint byte range.
    ///
    /// `on_progress` reports bytes-so-far within this range
    /// (monotonic from 0 up to `end - start`); the caller folds it
    /// into a tree-level cumulative atomic.
    pub async fn upload_range_with_progress_cancel<F>(
        &self,
        local: &Path,
        remote: &str,
        start: u64,
        end: u64,
        mut on_progress: F,
        cancel: Option<&CancellationToken>,
    ) -> Result<u64>
    where
        F: FnMut(u64) + Send,
    {
        use russh_sftp::protocol::OpenFlags;
        use std::io::SeekFrom;
        use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

        if start >= end {
            return Ok(0);
        }

        let mut local_file = tokio::fs::File::open(local).await.map_err(SshError::Io)?;
        local_file
            .seek(SeekFrom::Start(start))
            .await
            .map_err(SshError::Io)?;

        // WRITE | CREATE without TRUNCATE so workers don't fight
        // each other — first one to call may create, the rest just
        // open and seek to their assigned offset.
        let mut remote_file = self
            .inner
            .open_with_flags(remote.to_string(), OpenFlags::WRITE | OpenFlags::CREATE)
            .await
            .map_err(sftp_error)?;
        remote_file
            .seek(SeekFrom::Start(start))
            .await
            .map_err(SshError::Io)?;

        let range_len = end - start;
        let mut buf = vec![0u8; 64 * 1024];
        let mut written: u64 = 0;
        while written < range_len {
            if let Some(token) = cancel {
                if token.is_cancelled() {
                    let _ = remote_file.flush().await;
                    let _ = remote_file.shutdown().await;
                    return Err(SshError::Cancelled);
                }
            }
            let want = ((range_len - written) as usize).min(buf.len());
            let n = local_file
                .read(&mut buf[..want])
                .await
                .map_err(SshError::Io)?;
            if n == 0 {
                break;
            }
            remote_file
                .write_all(&buf[..n])
                .await
                .map_err(SshError::Io)?;
            written += n as u64;
            on_progress(written);
        }
        remote_file.flush().await.map_err(SshError::Io)?;
        let _ = remote_file.shutdown().await;
        Ok(written)
    }

    /// Mirror of [`Self::upload_range_with_progress_cancel`] for
    /// downloads — reads `remote[start..end]` and writes to `local`
    /// at offset `start`. The local file must already exist with at
    /// least `end` bytes capacity (caller pre-allocates via
    /// `set_len`); we open with `WRITE` (no truncate) so all
    /// workers can pwrite into disjoint regions.
    pub async fn download_range_with_progress_cancel<F>(
        &self,
        remote: &str,
        local: &Path,
        start: u64,
        end: u64,
        mut on_progress: F,
        cancel: Option<&CancellationToken>,
    ) -> Result<u64>
    where
        F: FnMut(u64) + Send,
    {
        use std::io::SeekFrom;
        use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

        if start >= end {
            return Ok(0);
        }

        let mut remote_file = self
            .inner
            .open(remote.to_string())
            .await
            .map_err(sftp_error)?;
        remote_file
            .seek(SeekFrom::Start(start))
            .await
            .map_err(SshError::Io)?;

        let mut local_file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(local)
            .await
            .map_err(SshError::Io)?;
        local_file
            .seek(SeekFrom::Start(start))
            .await
            .map_err(SshError::Io)?;

        let range_len = end - start;
        let mut buf = vec![0u8; 64 * 1024];
        let mut written: u64 = 0;
        while written < range_len {
            if let Some(token) = cancel {
                if token.is_cancelled() {
                    let _ = local_file.flush().await;
                    let _ = remote_file.shutdown().await;
                    return Err(SshError::Cancelled);
                }
            }
            let want = ((range_len - written) as usize).min(buf.len());
            let n = remote_file
                .read(&mut buf[..want])
                .await
                .map_err(SshError::Io)?;
            if n == 0 {
                break;
            }
            local_file
                .write_all(&buf[..n])
                .await
                .map_err(SshError::Io)?;
            written += n as u64;
            on_progress(written);
        }
        local_file.flush().await.map_err(SshError::Io)?;
        let _ = remote_file.shutdown().await;
        Ok(written)
    }

    /// Recursively upload `local_root` into `remote_root`, preserving
    /// the directory structure. Computes the total byte size in a
    /// first pass so `on_progress` can render an accurate bar across
    /// the whole tree — then streams each file via
    /// [`Self::upload_from_with_progress_blocking`].
    ///
    /// Symlinks are followed (same semantics as `tokio::fs::read`).
    /// Remote directory creation ignores "already exists" so partial
    /// re-runs resume cleanly.
    pub fn upload_tree_blocking<F>(
        &self,
        local_root: &Path,
        remote_root: &str,
        mut on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        // First pass: enumerate files + dirs, precompute total.
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

        on_progress(0, total);

        // Create the root + interior directories (best-effort: the
        // server may or may not allow mkdir on an existing dir;
        // we treat any error as non-fatal for pre-existing paths
        // and re-raise on file ops).
        let _ = self.create_dir_blocking(remote_root);
        for dir in &dirs {
            let _ = self.create_dir_blocking(dir);
        }

        let mut transferred: u64 = 0;
        for (local_path, remote_path, _size) in &files {
            let bytes = self.upload_from_with_progress_blocking(
                local_path,
                remote_path,
                |file_bytes, _file_total| {
                    on_progress(transferred + file_bytes, total);
                },
            )?;
            transferred = transferred.saturating_add(bytes);
            on_progress(transferred, total);
        }
        Ok(transferred)
    }

    /// Recursively download `remote_root` into `local_root`,
    /// preserving structure. Walks the remote tree, sums the total
    /// size, then streams each file via
    /// [`Self::download_to_with_progress_blocking`].
    pub fn download_tree_blocking<F>(
        &self,
        remote_root: &str,
        local_root: &Path,
        mut on_progress: F,
    ) -> Result<u64>
    where
        F: FnMut(u64, u64) + Send,
    {
        let mut files: Vec<(String, PathBuf, u64)> = Vec::new();
        let mut total: u64 = 0;
        runtime::shared().block_on(async {
            collect_remote_tree(self, remote_root, local_root, &mut files, &mut total).await
        })?;

        on_progress(0, total);
        if !local_root.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(local_root);
        }

        let mut transferred: u64 = 0;
        for (remote_path, local_path, _size) in &files {
            if let Some(parent) = local_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let bytes = self.download_to_with_progress_blocking(
                remote_path,
                local_path,
                |file_bytes, _file_total| {
                    on_progress(transferred + file_bytes, total);
                },
            )?;
            transferred = transferred.saturating_add(bytes);
            on_progress(transferred, total);
        }
        Ok(transferred)
    }

    // ── Directory / entry management ──────────────────────

    /// Create a directory at `path`. Non-recursive — parent
    /// must already exist. Returns an error if `path` already
    /// exists.
    pub async fn create_dir(&self, path: &str) -> Result<()> {
        self.inner
            .create_dir(path.to_string())
            .await
            .map_err(sftp_error)
    }

    /// Synchronous wrapper for [`Self::create_dir`].
    pub fn create_dir_blocking(&self, path: &str) -> Result<()> {
        runtime::shared().block_on(self.create_dir(path))
    }

    /// Remove a single file. Directories use
    /// [`Self::remove_dir`] instead.
    pub async fn remove_file(&self, path: &str) -> Result<()> {
        self.inner
            .remove_file(path.to_string())
            .await
            .map_err(sftp_error)
    }

    /// Synchronous wrapper for [`Self::remove_file`].
    pub fn remove_file_blocking(&self, path: &str) -> Result<()> {
        runtime::shared().block_on(self.remove_file(path))
    }

    /// Remove an empty directory. Non-recursive.
    pub async fn remove_dir(&self, path: &str) -> Result<()> {
        self.inner
            .remove_dir(path.to_string())
            .await
            .map_err(sftp_error)
    }

    /// Synchronous wrapper for [`Self::remove_dir`].
    pub fn remove_dir_blocking(&self, path: &str) -> Result<()> {
        runtime::shared().block_on(self.remove_dir(path))
    }

    /// Rename `from` to `to`. May also move across
    /// directories when the server supports it.
    pub async fn rename(&self, from: &str, to: &str) -> Result<()> {
        self.inner
            .rename(from.to_string(), to.to_string())
            .await
            .map_err(sftp_error)
    }

    /// Synchronous wrapper for [`Self::rename`].
    pub fn rename_blocking(&self, from: &str, to: &str) -> Result<()> {
        runtime::shared().block_on(self.rename(from, to))
    }

    /// Set POSIX permission bits on `path`. Only the low 12
    /// bits (`0o7777`) are preserved — callers pass file-mode
    /// octal like `0o644`; higher bits that encode the file
    /// type stay whatever the server already reports.
    pub async fn set_permissions(&self, path: &str, mode: u32) -> Result<()> {
        let mut attrs = FileAttributes::empty();
        attrs.permissions = Some(mode & 0o7777);
        self.inner
            .set_metadata(path.to_string(), attrs)
            .await
            .map_err(sftp_error)
    }

    /// Synchronous wrapper for [`Self::set_permissions`].
    pub fn set_permissions_blocking(&self, path: &str, mode: u32) -> Result<()> {
        runtime::shared().block_on(self.set_permissions(path, mode))
    }

    /// Create an empty file at `path`, opening and immediately
    /// closing the handle. Mirrors `touch` for missing files;
    /// will truncate an existing file to zero length on servers
    /// that treat `SSH_FXP_OPEN | CREAT | TRUNC` that way, so
    /// callers should stat first if they need "fail on exist".
    pub async fn create_file(&self, path: &str) -> Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut file = self
            .inner
            .create(path.to_string())
            .await
            .map_err(sftp_error)?;
        let _ = file.shutdown().await;
        Ok(())
    }

    /// Synchronous wrapper for [`Self::create_file`].
    pub fn create_file_blocking(&self, path: &str) -> Result<()> {
        runtime::shared().block_on(self.create_file(path))
    }

    /// Canonicalize a (possibly relative or symlinked) path to
    /// an absolute form. The common use is "pwd": call
    /// `canonicalize(".")` right after opening the session.
    pub async fn canonicalize(&self, path: &str) -> Result<String> {
        self.inner
            .canonicalize(path.to_string())
            .await
            .map_err(sftp_error)
    }

    /// Synchronous wrapper for [`Self::canonicalize`].
    pub fn canonicalize_blocking(&self, path: &str) -> Result<String> {
        runtime::shared().block_on(self.canonicalize(path))
    }
}

/// Translate russh-sftp's error type into our unified
/// `SshError`. russh-sftp errors don't have a clean direct
/// mapping into `SshError::Protocol` (the inner types differ),
/// so we format them as strings and route through
/// `SshError::InvalidConfig` for non-I/O cases and
/// `SshError::Io` for the rest.
/// Recursively walk a local directory, collecting files that should
/// be uploaded and the directories that should be `mkdir`'d on the
/// remote side. Populates `total` with the sum of file sizes so the
/// caller can render an accurate progress bar.
///
/// `local_root` is the folder the user picked; `current` is what
/// we're iterating in this recursion step. `remote_root` is the
/// destination path — remote paths are derived from the relative
/// portion of each local path.
pub(super) fn collect_local_tree(
    local_root: &Path,
    current: &Path,
    remote_root: &str,
    files: &mut Vec<(PathBuf, String, u64)>,
    dirs: &mut Vec<String>,
    total: &mut u64,
) -> Result<()> {
    let read = std::fs::read_dir(current).map_err(SshError::Io)?;
    for entry_result in read {
        let entry = entry_result.map_err(SshError::Io)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(SshError::Io)?;
        let rel = path
            .strip_prefix(local_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let remote_path = if rel.is_empty() {
            remote_root.to_string()
        } else {
            format!("{}/{}", remote_root.trim_end_matches('/'), rel)
        };
        if file_type.is_dir() {
            dirs.push(remote_path);
            collect_local_tree(local_root, &path, remote_root, files, dirs, total)?;
        } else if file_type.is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            *total = total.saturating_add(size);
            files.push((path, remote_path, size));
        }
    }
    Ok(())
}

/// Recursively walk a remote directory, collecting files to download.
/// `remote_root` is the remote folder the user picked; `local_root`
/// is the destination local directory. Local paths preserve the
/// relative tree under `local_root`.
pub(super) async fn collect_remote_tree(
    sftp: &SftpClient,
    remote_root: &str,
    local_root: &Path,
    files: &mut Vec<(String, PathBuf, u64)>,
    total: &mut u64,
) -> Result<()> {
    let mut stack: Vec<(String, PathBuf)> =
        vec![(remote_root.to_string(), local_root.to_path_buf())];
    while let Some((remote_dir, local_dir)) = stack.pop() {
        let entries = sftp.list_dir(&remote_dir).await?;
        for entry in entries {
            let target_local = local_dir.join(&entry.name);
            if entry.is_dir {
                stack.push((entry.path.clone(), target_local));
            } else {
                *total = total.saturating_add(entry.size);
                files.push((entry.path, target_local, entry.size));
            }
        }
    }
    Ok(())
}

/// Pick the most useful owner / group display string from the
/// SFTP attributes. Servers running OpenSSH on Linux typically
/// fill in the named field; minimal SFTP servers and servers
/// configured against a stripped `/etc/passwd` only return the
/// numeric id. We prefer the name; fall back to the numeric id
/// formatted as a string; otherwise `None`.
fn pick_owner(name: Option<&str>, id: Option<u32>) -> Option<String> {
    if let Some(s) = name.filter(|s| !s.is_empty()) {
        return Some(s.to_string());
    }
    id.map(|n| n.to_string())
}

fn sftp_error(e: russh_sftp::client::error::Error) -> SshError {
    // Any kind of transport error we treat as ChannelClosed;
    // everything else becomes a stringified config-ish error
    // since the UI only cares about the human-readable
    // message.
    use russh_sftp::client::error::Error as E;
    match e {
        E::UnexpectedBehavior(_) | E::UnexpectedPacket => SshError::ChannelClosed,
        other => SshError::InvalidConfig(format!("sftp: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_file_entry_round_trips_through_json() {
        let entry = RemoteFileEntry {
            name: "app.log".to_string(),
            path: "/var/log/app.log".to_string(),
            is_dir: false,
            is_link: false,
            size: 4096,
            modified: Some(1_700_000_000),
            permissions: Some(0o644),
            owner: Some("deploy".to_string()),
            group: Some("deploy".to_string()),
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: RemoteFileEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry, back);
    }

    #[test]
    fn pick_owner_prefers_named_over_numeric() {
        assert_eq!(pick_owner(Some("alice"), Some(1000)), Some("alice".into()));
        assert_eq!(pick_owner(None, Some(1000)), Some("1000".into()));
        assert_eq!(pick_owner(Some(""), Some(0)), Some("0".into()));
        assert_eq!(pick_owner(None, None), None);
    }

    #[test]
    fn remote_file_entry_directory_vs_file_sort_order() {
        // The list_dir sort rule is "directories first, then
        // case-insensitive alphabetical". Verify both halves
        // independently of any live SFTP session.
        let mut entries = [
            RemoteFileEntry {
                name: "zeta.txt".into(),
                path: "/zeta.txt".into(),
                is_dir: false,
                is_link: false,
                size: 1,
                modified: None,
                permissions: None,
                owner: None,
                group: None,
            },
            RemoteFileEntry {
                name: "Alpha".into(),
                path: "/Alpha".into(),
                is_dir: true,
                is_link: false,
                size: 0,
                modified: None,
                permissions: None,
                owner: None,
                group: None,
            },
            RemoteFileEntry {
                name: "apple.md".into(),
                path: "/apple.md".into(),
                is_dir: false,
                is_link: false,
                size: 12,
                modified: None,
                permissions: None,
                owner: None,
                group: None,
            },
            RemoteFileEntry {
                name: "beta".into(),
                path: "/beta".into(),
                is_dir: true,
                is_link: false,
                size: 0,
                modified: None,
                permissions: None,
                owner: None,
                group: None,
            },
        ];
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "beta", "apple.md", "zeta.txt"]);
    }

    #[test]
    fn sftp_error_maps_transport_to_channel_closed() {
        use russh_sftp::client::error::Error as E;
        let transport = sftp_error(E::UnexpectedPacket);
        assert!(matches!(transport, SshError::ChannelClosed));
    }
}
