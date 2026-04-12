//! Persisted SSH connection list.
//!
//! ## What lives here
//!
//! [`ConnectionStore`] is the on-disk representation of the
//! sidebar list of saved SSH connections. It is a thin
//! `serde_json` wrapper around `Vec<SshConfig>` plus a schema
//! version field, with atomic load/save through
//! [`crate::paths::connections_file`].
//!
//! ## What does NOT live here
//!
//! Secrets. [`SshConfig::auth`] only ever holds opaque
//! `credential_id` strings — the actual passwords / passphrases
//! are stored in the OS keychain via [`crate::credentials`] and
//! looked up by id at connection time. The persisted JSON
//! therefore contains nothing the user couldn't safely sync
//! across machines or commit to a private dotfiles repo.
//!
//! The [`SshConfig::AuthMethod::DirectPassword`] variant is
//! `#[serde(skip)]` so even an accidental round-trip of a
//! test-only config can't leak credentials to disk.
//!
//! ## File format
//!
//! ```json
//! {
//!   "version": 1,
//!   "connections": [
//!     {
//!       "name": "prod",
//!       "host": "db.example.com",
//!       "port": 22,
//!       "user": "deploy",
//!       "auth": { "kind": "keychain_password",
//!                 "credential_id": "pier-x.0d3a..." },
//!       "connect_timeout_secs": 10,
//!       "tags": []
//!     }
//!   ]
//! }
//! ```
//!
//! The `version` field is bumped on any breaking schema change.
//! M3c2 ships v1; future migrations get explicit `match` arms in
//! [`ConnectionStore::load_from_path`].
//!
//! ## Atomicity
//!
//! [`ConnectionStore::save_to_path`] writes to a sibling
//! `.connections.json.tmp` file then atomically renames it onto
//! the real path, so a crash mid-write can never leave the
//! sidebar in a half-baked state. Both POSIX and Win32 rename
//! into an existing file are atomic; the temp file lives in the
//! same directory so the rename is a single inode swap.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;
use crate::ssh::SshConfig;

/// Current on-disk schema version. Bumped on any breaking
/// change to the JSON shape.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Errors that can occur loading or saving the connections file.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionStoreError {
    /// I/O error reading or writing the file.
    #[error("connections store I/O: {0}")]
    Io(#[from] io::Error),

    /// JSON parse error — the file exists but is malformed.
    #[error("connections store JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// The file's `version` field is from the future and we
    /// don't know how to migrate. Caller should typically rename
    /// the file aside and start fresh rather than overwrite it.
    #[error("connections store version {found} > supported {supported}")]
    FutureVersion {
        /// The version stamped on the file we just read.
        found: u32,
        /// The highest version this build of pier-core understands.
        supported: u32,
    },

    /// `paths::connections_file()` returned None — no usable
    /// home directory.
    #[error("no usable application data directory")]
    NoDataDir,
}

/// Top-level on-disk representation. Holds a versioned
/// connections list. New fields are appended in future versions
/// behind `#[serde(default)]` so older files load forward.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionStore {
    /// Schema version stamped on the file. Always written as
    /// [`CURRENT_SCHEMA_VERSION`]; on read, used to dispatch
    /// version-specific migrations.
    pub version: u32,
    /// The actual connections list, in display order.
    #[serde(default)]
    pub connections: Vec<SshConfig>,
}

impl Default for ConnectionStore {
    fn default() -> Self {
        Self {
            version: CURRENT_SCHEMA_VERSION,
            connections: Vec::new(),
        }
    }
}

impl ConnectionStore {
    /// Build an empty store stamped at the current schema version.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the store from the standard location resolved by
    /// [`crate::paths::connections_file`]. Returns the default
    /// (empty) store if the file does not yet exist.
    pub fn load_default() -> Result<Self, ConnectionStoreError> {
        let path = paths::connections_file().ok_or(ConnectionStoreError::NoDataDir)?;
        Self::load_from_path(&path)
    }

    /// Load from an explicit path. Used by tests; production
    /// callers should use [`Self::load_default`].
    ///
    /// Missing file → `Ok(Self::default())`. Malformed file →
    /// [`ConnectionStoreError::Json`]. Future schema →
    /// [`ConnectionStoreError::FutureVersion`].
    pub fn load_from_path(path: &Path) -> Result<Self, ConnectionStoreError> {
        match fs::read(path) {
            Ok(bytes) => {
                let store: Self = serde_json::from_slice(&bytes)?;
                if store.version > CURRENT_SCHEMA_VERSION {
                    return Err(ConnectionStoreError::FutureVersion {
                        found: store.version,
                        supported: CURRENT_SCHEMA_VERSION,
                    });
                }
                // Future migrations land here as `match store.version`.
                Ok(store)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persist the store to the standard location. Creates the
    /// containing directory if it doesn't yet exist.
    pub fn save_default(&self) -> Result<(), ConnectionStoreError> {
        let path = paths::connections_file().ok_or(ConnectionStoreError::NoDataDir)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        self.save_to_path(&path)
    }

    /// Persist to an explicit path, atomically: write to a
    /// sibling `.tmp` then rename. The version field is forced
    /// to [`CURRENT_SCHEMA_VERSION`] before serialization.
    pub fn save_to_path(&self, path: &Path) -> Result<(), ConnectionStoreError> {
        // Always stamp the latest version on save, even if we
        // loaded an older file — the in-memory representation
        // is always converted forward by load_from_path.
        let stamped = Self {
            version: CURRENT_SCHEMA_VERSION,
            connections: self.connections.clone(),
        };
        let json = serde_json::to_vec_pretty(&stamped)?;

        let tmp_path = tmp_path_for(path);
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Append a connection. The caller is responsible for ensuring
    /// the credential id (if any) has already been stored in the
    /// keychain — this method does not touch credentials.
    pub fn add(&mut self, config: SshConfig) {
        self.connections.push(config);
    }

    /// Remove the connection at `index`. Out-of-range indices
    /// are silently ignored so the C ABI doesn't have to deal
    /// with errors for a benign no-op.
    pub fn remove(&mut self, index: usize) -> Option<SshConfig> {
        if index < self.connections.len() {
            Some(self.connections.remove(index))
        } else {
            None
        }
    }
}

/// Compute the temp-file sibling path used by atomic save.
/// `.tmp` is appended to the file name (preserving the parent
/// directory) so the rename is single-directory-atomic.
fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    match path.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::{AuthMethod, SshConfig};
    use std::env::temp_dir;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Per-test temp file in the system temp dir. Each test gets
    /// a unique name so parallel runs don't collide.
    fn fresh_tmp(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        temp_dir().join(format!("pier-x-test-conn-{label}-{pid}-{n}.json"))
    }

    fn make_config(name: &str, credential_id: &str) -> SshConfig {
        let mut c = SshConfig::new(name, format!("{name}.example.com"), "deploy");
        c.port = 22;
        c.auth = AuthMethod::KeychainPassword {
            credential_id: credential_id.to_string(),
        };
        c
    }

    #[test]
    fn empty_store_round_trips() {
        let path = fresh_tmp("empty");
        let store = ConnectionStore::new();
        store.save_to_path(&path).expect("save");
        let loaded = ConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded, store);
        // cleanup
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn nonexistent_file_returns_default() {
        let path = fresh_tmp("missing");
        // Don't create the file. Loading must give us a default.
        assert!(!path.exists());
        let loaded = ConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded, ConnectionStore::default());
    }

    #[test]
    fn store_with_connections_round_trips() {
        let path = fresh_tmp("two");
        let mut store = ConnectionStore::new();
        store.add(make_config("prod", "pier-x.cred-1"));
        store.add(make_config("staging", "pier-x.cred-2"));
        store.save_to_path(&path).expect("save");

        let loaded = ConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded.version, CURRENT_SCHEMA_VERSION);
        assert_eq!(loaded.connections.len(), 2);
        assert_eq!(loaded.connections[0].name, "prod");
        assert_eq!(loaded.connections[1].name, "staging");
        match &loaded.connections[0].auth {
            AuthMethod::KeychainPassword { credential_id } => {
                assert_eq!(credential_id, "pier-x.cred-1");
            }
            other => panic!("expected KeychainPassword, got {other:?}"),
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let path = fresh_tmp("future");
        let json = serde_json::json!({
            "version": CURRENT_SCHEMA_VERSION + 9_999,
            "connections": []
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let err = ConnectionStore::load_from_path(&path)
            .expect_err("future-version file must be rejected");
        match err {
            ConnectionStoreError::FutureVersion { found, supported } => {
                assert_eq!(found, CURRENT_SCHEMA_VERSION + 9_999);
                assert_eq!(supported, CURRENT_SCHEMA_VERSION);
            }
            other => panic!("expected FutureVersion, got {other:?}"),
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn malformed_json_is_rejected() {
        let path = fresh_tmp("garbage");
        fs::write(&path, b"this is not json").unwrap();
        let err = ConnectionStore::load_from_path(&path).expect_err("garbage must be rejected");
        assert!(matches!(err, ConnectionStoreError::Json(_)));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn add_then_remove_round_trips_in_memory() {
        let mut store = ConnectionStore::new();
        store.add(make_config("a", "id-a"));
        store.add(make_config("b", "id-b"));
        store.add(make_config("c", "id-c"));
        let removed = store.remove(1).expect("removed b");
        assert_eq!(removed.name, "b");
        assert_eq!(store.connections.len(), 2);
        assert_eq!(store.connections[0].name, "a");
        assert_eq!(store.connections[1].name, "c");
    }

    #[test]
    fn remove_out_of_range_is_safe_noop() {
        let mut store = ConnectionStore::new();
        store.add(make_config("only", "id-only"));
        assert!(store.remove(99).is_none());
        assert_eq!(store.connections.len(), 1);
    }

    #[test]
    fn save_is_atomic_no_temp_file_left_behind() {
        let path = fresh_tmp("atomic");
        let mut store = ConnectionStore::new();
        store.add(make_config("only", "id-only"));
        store.save_to_path(&path).expect("save");

        // The .tmp sibling should NOT exist after a successful save.
        let tmp = tmp_path_for(&path);
        assert!(
            !tmp.exists(),
            "atomic save left a stray temp file behind: {tmp:?}",
        );
        assert!(path.exists(), "atomic save did not produce final file");
        let _ = fs::remove_file(&path);
    }
}
