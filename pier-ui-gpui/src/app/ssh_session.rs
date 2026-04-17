#![allow(dead_code)]
//! SSH session backing the right-panel SFTP browser (and, eventually, the
//! Docker / Logs / DB modes that need an `exec` channel into the same host).
//!
//! Each connection the user clicks in the Servers list spawns one of these.
//! The session is **lazy-connected**: nothing happens until the SFTP view
//! actually asks for a directory listing — then we run
//! [`SshSession::connect_blocking`] + [`SshSession::open_sftp_blocking`] on
//! the calling thread. First-connect freezes the UI for the duration of the
//! handshake (typically <2s on LAN); a follow-on PR will move this to a
//! background task with a `Connecting…` placeholder.
//!
//! Auth coverage:
//!   - `AuthMethod::Agent`            — handled by russh
//!   - `AuthMethod::DirectPassword`   — handled by russh
//!   - `AuthMethod::KeychainPassword` — pier-core looks up the OS keychain
//!   - `AuthMethod::PublicKeyFile`    — pier-core reads file + opt. passphrase

use std::path::PathBuf;

use pier_core::ssh::{HostKeyVerifier, SftpClient, SshConfig, SshSession};

/// Where the SFTP root lands on first connect — `~` is the SSH spec's
/// shorthand for "user's home directory" and SFTP servers honour it.
pub const DEFAULT_REMOTE_ROOT: &str = ".";

#[derive(Clone, Debug)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_link: bool,
    pub size: u64,
}

#[derive(Debug, Default)]
pub enum ConnectStatus {
    #[default]
    Idle,
    Connected,
    Failed(String),
}

pub struct SshSessionState {
    pub config: SshConfig,
    /// Held to keep the underlying russh handle alive across SFTP ops.
    /// Boxed inside an Option so first-connect can populate it without
    /// re-allocating `self`.
    session: Option<SshSession>,
    sftp: Option<SftpClient>,
    pub cwd: PathBuf,
    pub entries: Vec<RemoteEntry>,
    pub status: ConnectStatus,
    pub last_error: Option<String>,
}

impl SshSessionState {
    pub fn new(config: SshConfig) -> Self {
        Self {
            config,
            session: None,
            sftp: None,
            cwd: PathBuf::from(DEFAULT_REMOTE_ROOT),
            entries: Vec::new(),
            status: ConnectStatus::Idle,
            last_error: None,
        }
    }

    /// Lazy-connect + open SFTP if not yet open. Returns the cached
    /// SftpClient on success.
    fn ensure_sftp(&mut self) -> Result<&SftpClient, String> {
        if self.sftp.is_some() {
            return Ok(self.sftp.as_ref().unwrap());
        }

        let verifier = HostKeyVerifier::default();
        let session = SshSession::connect_blocking(&self.config, verifier)
            .map_err(|e| e.to_string())?;
        let sftp = session.open_sftp_blocking().map_err(|e| e.to_string())?;
        self.session = Some(session);
        self.sftp = Some(sftp);
        self.status = ConnectStatus::Connected;
        Ok(self.sftp.as_ref().unwrap())
    }

    /// Fetch listings for the current `cwd` and refresh `entries`.
    pub fn refresh(&mut self) {
        let path_str = self
            .cwd
            .to_str()
            .map(str::to_string)
            .unwrap_or_else(|| DEFAULT_REMOTE_ROOT.to_string());

        match self.ensure_sftp() {
            Ok(sftp) => match sftp.list_dir_blocking(&path_str) {
                Ok(remote_entries) => {
                    self.entries = remote_entries
                        .into_iter()
                        .map(|e| RemoteEntry {
                            name: e.name,
                            path: e.path,
                            is_dir: e.is_dir,
                            is_link: e.is_link,
                            size: e.size,
                        })
                        // Hide dotfiles by default — matches LocalFileView.
                        .filter(|e| !e.name.starts_with('.'))
                        .collect();
                    self.entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                    });
                    self.last_error = None;
                }
                Err(err) => {
                    self.last_error = Some(format!("list_dir({path_str}): {err}"));
                    self.entries.clear();
                }
            },
            Err(err) => {
                self.status = ConnectStatus::Failed(err.clone());
                self.last_error = Some(err);
                self.entries.clear();
            }
        }
    }

    pub fn navigate_to(&mut self, path: PathBuf) {
        self.cwd = path;
        self.refresh();
    }

    pub fn cd_up(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            let parent_buf = parent.to_path_buf();
            if !parent_buf.as_os_str().is_empty() {
                self.cwd = parent_buf;
            } else {
                self.cwd = PathBuf::from("/");
            }
            self.refresh();
        }
    }

    pub fn is_connected(&self) -> bool {
        matches!(self.status, ConnectStatus::Connected)
    }
}
