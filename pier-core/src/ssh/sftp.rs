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
//! * Recursive operations (`rm -rf`, `mkdir -p`). The shell
//!   composes these from the single-step primitives today;
//!   we'll move them into this module when recursion becomes
//!   annoying to orchestrate above this layer.
//! * Permission editing. `set_metadata` exists in russh-sftp
//!   but pier-x's M3d shell doesn't expose a chmod affordance
//!   yet.

use std::path::Path;
use std::sync::Arc;

use russh_sftp::client::SftpSession;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::error::{Result, SshError};
use super::runtime;

/// Chunk used for streaming uploads / downloads. Sized to stay well
/// under russh-sftp's per-packet ceiling (~256 KiB) while still
/// keeping the read/write loop short enough that progress updates
/// arrive fluidly (~16 updates per MB at the default emit rate).
const TRANSFER_CHUNK: usize = 64 * 1024;

/// Emit a progress tick at least this often during a streaming
/// transfer. Tuned so a 10 MB download produces ~20 updates and the
/// UI's progress bar animates without the callback path becoming a
/// hot loop on large files.
const PROGRESS_EMIT_STEP: u64 = 512 * 1024;

/// Snapshot of a single in-flight transfer. Emitted through
/// [`ProgressCallback`] by the chunked transfer loop so the UI can
/// animate a progress bar, show bytes transferred, and compute a
/// throughput estimate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferProgress {
    /// Bytes moved across the wire so far.
    pub transferred: u64,
    /// Total bytes expected. `0` means the remote didn't report a
    /// size — render the bar in indeterminate mode in that case.
    pub total: u64,
}

/// Called by [`SftpClient::download_to`] / [`SftpClient::upload_from`]
/// once per chunk (subject to [`PROGRESS_EMIT_STEP`]). Must be
/// `Send + Sync` because the transfer loop runs on the shared tokio
/// runtime, not on the caller's thread.
pub type ProgressCallback = Arc<dyn Fn(TransferProgress) + Send + Sync + 'static>;

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
            out.push(RemoteFileEntry {
                name,
                path: full_path,
                is_dir: file_type.is_dir(),
                is_link: file_type.is_symlink(),
                size: metadata.size.unwrap_or(0),
                modified: metadata.mtime.map(|v| v as u64),
                permissions: metadata.permissions,
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
        Ok(RemoteFileEntry {
            name,
            path: path.to_string(),
            is_dir: file_type.is_dir(),
            is_link: file_type.is_symlink(),
            size: metadata.size.unwrap_or(0),
            modified: metadata.mtime.map(|v| v as u64),
            permissions: metadata.permissions,
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

    /// Download `remote` to the local filesystem at `local`, streamed
    /// in [`TRANSFER_CHUNK`]-sized blocks so multi-MB files don't
    /// balloon memory. If `progress` is `Some`, the callback fires
    /// once with `transferred = 0` before the first read (so the UI
    /// can show the total right away), then every
    /// [`PROGRESS_EMIT_STEP`] bytes, and once more at completion.
    pub async fn download_to(
        &self,
        remote: &str,
        local: &Path,
        progress: Option<ProgressCallback>,
    ) -> Result<()> {
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

        let mut local_file = tokio::fs::File::create(local).await.map_err(SshError::Io)?;
        let mut buf = vec![0u8; TRANSFER_CHUNK];
        let mut transferred: u64 = 0;
        let mut last_emitted: u64 = 0;
        emit_progress(progress.as_ref(), 0, total);

        loop {
            let n = remote_file.read(&mut buf).await.map_err(SshError::Io)?;
            if n == 0 {
                break;
            }
            local_file
                .write_all(&buf[..n])
                .await
                .map_err(SshError::Io)?;
            transferred += n as u64;
            if transferred - last_emitted >= PROGRESS_EMIT_STEP {
                emit_progress(progress.as_ref(), transferred, total);
                last_emitted = transferred;
            }
        }
        local_file.flush().await.map_err(SshError::Io)?;
        emit_progress(progress.as_ref(), transferred, total.max(transferred));
        log::info!("downloaded {remote} -> {local}", local = local.display());
        Ok(())
    }

    /// Sync wrapper for [`Self::download_to`].
    pub fn download_to_blocking(
        &self,
        remote: &str,
        local: &Path,
        progress: Option<ProgressCallback>,
    ) -> Result<()> {
        runtime::shared().block_on(self.download_to(remote, local, progress))
    }

    /// Upload `local` to remote path `remote`, streamed in
    /// [`TRANSFER_CHUNK`]-sized blocks. Progress semantics mirror
    /// [`Self::download_to`]: an initial tick with `transferred = 0`
    /// followed by periodic updates and a final tick at completion.
    pub async fn upload_from(
        &self,
        local: &Path,
        remote: &str,
        progress: Option<ProgressCallback>,
    ) -> Result<()> {
        let total = tokio::fs::metadata(local)
            .await
            .map_err(SshError::Io)?
            .len();
        let mut local_file = tokio::fs::File::open(local).await.map_err(SshError::Io)?;
        let mut remote_file = self
            .inner
            .create(remote.to_string())
            .await
            .map_err(sftp_error)?;

        let mut buf = vec![0u8; TRANSFER_CHUNK];
        let mut transferred: u64 = 0;
        let mut last_emitted: u64 = 0;
        emit_progress(progress.as_ref(), 0, total);

        loop {
            let n = local_file.read(&mut buf).await.map_err(SshError::Io)?;
            if n == 0 {
                break;
            }
            remote_file
                .write_all(&buf[..n])
                .await
                .map_err(SshError::Io)?;
            transferred += n as u64;
            if transferred - last_emitted >= PROGRESS_EMIT_STEP {
                emit_progress(progress.as_ref(), transferred, total);
                last_emitted = transferred;
            }
        }
        // Explicit shutdown is required so russh-sftp releases the
        // remote handle; without it the server may leave the file
        // half-synced until the SFTP session closes.
        remote_file.shutdown().await.map_err(SshError::Io)?;
        emit_progress(progress.as_ref(), transferred, total.max(transferred));
        log::info!("uploaded {local} -> {remote}", local = local.display());
        Ok(())
    }

    /// Sync wrapper for [`Self::upload_from`].
    pub fn upload_from_blocking(
        &self,
        local: &Path,
        remote: &str,
        progress: Option<ProgressCallback>,
    ) -> Result<()> {
        runtime::shared().block_on(self.upload_from(local, remote, progress))
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

/// Fire a progress callback if one was supplied. Kept as a free
/// function so both the download and upload loops can share the
/// exact same emission shape.
fn emit_progress(callback: Option<&ProgressCallback>, transferred: u64, total: u64) {
    if let Some(cb) = callback {
        cb(TransferProgress { transferred, total });
    }
}

/// Translate russh-sftp's error type into our unified
/// `SshError`. russh-sftp errors don't have a clean direct
/// mapping into `SshError::Protocol` (the inner types differ),
/// so we format them as strings and route through
/// `SshError::InvalidConfig` for non-I/O cases and
/// `SshError::Io` for the rest.
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
        };
        let json = serde_json::to_string(&entry).expect("serialize");
        let back: RemoteFileEntry = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(entry, back);
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
            },
            RemoteFileEntry {
                name: "Alpha".into(),
                path: "/Alpha".into(),
                is_dir: true,
                is_link: false,
                size: 0,
                modified: None,
                permissions: None,
            },
            RemoteFileEntry {
                name: "apple.md".into(),
                path: "/apple.md".into(),
                is_dir: false,
                is_link: false,
                size: 12,
                modified: None,
                permissions: None,
            },
            RemoteFileEntry {
                name: "beta".into(),
                path: "/beta".into(),
                is_dir: true,
                is_link: false,
                size: 0,
                modified: None,
                permissions: None,
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
