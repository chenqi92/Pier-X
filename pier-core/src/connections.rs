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

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::credentials;
use crate::paths;
use crate::ssh::config::{DbCredential, DbCredentialSource, DbKind, DbPasswordStorage};
use crate::ssh::SshConfig;

/// Process-local plaintext cache for DB credentials whose keyring
/// write was silently dropped (macOS sandbox, Linux without a
/// running secret-service daemon, Windows CM group policy, …).
///
/// `save_db_credential` writes a plaintext entry here whenever
/// `store_password` has to fall back to [`DbPasswordStorage::Direct`];
/// `resolve_db_credential` reads it back before returning, so a DB
/// panel asking for the password can actually connect.
///
/// Without this cache the `Direct` variant effectively forgets the
/// password as soon as `save_default()` round-trips the entry
/// through YAML, because [`DbPasswordStorage::Direct::password`] is
/// `#[serde(skip)]` — and then the panel tries to AUTH with an empty
/// string, producing the cryptic `NOAUTH` / `Access denied` errors
/// that look like "wrong credentials" to the user.
///
/// The cache is deliberately process-local and in-memory only:
/// plaintext never touches disk. The user still needs to re-enter
/// the password on the next launch if the keyring refuses to hold
/// it, but within one session the Save → Connect path works.
fn direct_password_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_put(key: &str, value: String) {
    if let Ok(mut guard) = direct_password_cache().lock() {
        guard.insert(key.to_string(), value);
    }
}

fn cache_get(key: &str) -> Option<String> {
    direct_password_cache()
        .lock()
        .ok()
        .and_then(|g| g.get(key).cloned())
}

fn cache_forget(key: &str) {
    if let Ok(mut guard) = direct_password_cache().lock() {
        guard.remove(key);
    }
}

/// True when a DB credential has, or once had, a stored password.
///
/// Preferred over [`DbPasswordStorage::is_present`] in the Tauri
/// view layer. Semantics differ from `is_present` in one place
/// that matters:
///
/// * For [`DbPasswordStorage::Direct`] this returns `true` based
///   on the variant tag alone, even when the runtime plaintext
///   was dropped across a YAML round-trip (the `password` field
///   is `#[serde(skip)]`). The variant tag is enough to know
///   "the user saved this credential with a password"; whether
///   the password is still resolvable in *this* process is a
///   separate question handled by `resolve_db_credential`.
///
/// The frontend uses this to decide whether to show the "Saved
/// password unavailable — enter it manually" fallback vs. a plain
/// "no password needed" flow. Returning `false` for a silently-
/// dropped Direct password would send the panel down the
/// passwordless branch and surface a cryptic `NOAUTH` from the
/// server.
pub fn password_available(cred: &DbCredential) -> bool {
    match &cred.password {
        DbPasswordStorage::Keyring { credential_id } => !credential_id.is_empty(),
        DbPasswordStorage::Direct { .. } => true,
        DbPasswordStorage::None => false,
    }
}

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
    /// are silently ignored so callers can treat removal as an
    /// idempotent no-op.
    pub fn remove(&mut self, index: usize) -> Option<SshConfig> {
        if index < self.connections.len() {
            Some(self.connections.remove(index))
        } else {
            None
        }
    }

    /// Atomic reorder + group-reassign: rewrite the connections
    /// list to the given permutation `order` (each entry is an
    /// old index) and apply `groups` as the new group labels in
    /// the same slot. Group order is derived from first-appearance
    /// in the resulting list, so reordering groups is done by
    /// arranging all their members contiguously in the desired
    /// order. Lengths of `order` and `groups` must match and
    /// `order` must be a permutation of `0..connections.len()`.
    pub fn reorder_with_groups(
        &mut self,
        order: &[usize],
        groups: &[Option<String>],
    ) -> Result<(), ConnectionStoreError> {
        if order.len() != self.connections.len() || groups.len() != order.len() {
            return Err(ConnectionStoreError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reorder: order / groups length must equal connections length",
            )));
        }
        let mut seen = vec![false; self.connections.len()];
        for &idx in order {
            if idx >= self.connections.len() || seen[idx] {
                return Err(ConnectionStoreError::Io(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "reorder: order must be a permutation",
                )));
            }
            seen[idx] = true;
        }
        let old = std::mem::take(&mut self.connections);
        let mut next: Vec<SshConfig> = Vec::with_capacity(old.len());
        // Move values out of `old` in the order given. Using `Option`
        // to track which slots are already taken.
        let mut old_opt: Vec<Option<SshConfig>> = old.into_iter().map(Some).collect();
        for (slot, &idx) in order.iter().enumerate() {
            let mut cfg = old_opt[idx]
                .take()
                .expect("permutation guarantees first-take");
            let new_group = groups[slot]
                .as_ref()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            cfg.group = new_group;
            next.push(cfg);
        }
        self.connections = next;
        Ok(())
    }

    /// Rename a group: every connection whose `group` equals
    /// `from` gets its group set to `to` (or `None` if `to` is
    /// empty). Returns the number of connections updated.
    pub fn rename_group(&mut self, from: &str, to: Option<&str>) -> usize {
        let target = to
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(String::from);
        let mut touched = 0usize;
        for c in self.connections.iter_mut() {
            let matches = c
                .group
                .as_deref()
                .map(|s| s == from)
                .unwrap_or_else(|| from.is_empty());
            if matches {
                c.group = target.clone();
                touched += 1;
            }
        }
        touched
    }
}

/// Input shape for [`save_db_credential`] — the caller fills
/// everything except `id` (assigned here) and `password`
/// (supplied separately so it never rides in a `Deserialize`
/// surface). `password` of `None` means "no password"
/// (passwordless) and maps to [`DbPasswordStorage::None`].
#[derive(Debug, Clone)]
pub struct NewDbCredential {
    /// Which panel it belongs to.
    pub kind: DbKind,
    /// User-facing label.
    pub label: String,
    /// Remote-side host (unused for SQLite).
    pub host: String,
    /// Remote-side port (unused for SQLite).
    pub port: u16,
    /// DB user (empty for Redis/Sqlite).
    pub user: String,
    /// Default database / schema / Redis DB index.
    pub database: Option<String>,
    /// Absolute remote path when `kind == Sqlite`.
    pub sqlite_path: Option<String>,
    /// Mark as the favourite for its kind on this profile.
    pub favorite: bool,
    /// Where the credential came from (detection / manual).
    pub source: DbCredentialSource,
}

/// Patch for [`update_db_credential`]. Every field is
/// optional — only supplied fields are applied.
#[derive(Debug, Clone, Default)]
pub struct DbCredentialPatch {
    /// Renames the credential in the UI picker.
    pub label: Option<String>,
    /// Change the host we dial on the remote side.
    pub host: Option<String>,
    /// Change the remote port.
    pub port: Option<u16>,
    /// Change the DB user.
    pub user: Option<String>,
    /// Change the default database (wrap a `Some("")` to clear).
    pub database: Option<Option<String>>,
    /// Change the remote SQLite path.
    pub sqlite_path: Option<Option<String>>,
    /// Flip the favourite bit.
    pub favorite: Option<bool>,
}

/// Resolved credential ready to connect with. The `password`
/// is populated when the stored variant is `Keyring` or
/// `Direct`; `None` means passwordless.
#[derive(Debug, Clone)]
pub struct ResolvedDbCredential {
    /// Persisted credential metadata.
    pub credential: DbCredential,
    /// Password in memory. Never logged. Caller must drop it
    /// as soon as the connection attempt completes.
    pub password: Option<String>,
}

/// Errors specific to the DB credential helpers.
#[derive(Debug, thiserror::Error)]
pub enum DbCredentialError {
    /// Connection index out of range.
    #[error("ssh connection index {0} out of range")]
    ConnectionIndex(usize),
    /// Credential id not found on that connection.
    #[error("db credential id {0} not found on connection")]
    NotFound(String),
    /// Persistence failed.
    #[error("connection store error: {0}")]
    Store(#[from] ConnectionStoreError),
    /// OS keyring failed (other than "silently dropped" which
    /// falls back to `Direct`).
    #[error("credential store error: {0}")]
    Credential(#[from] credentials::CredentialError),
}

/// Generate a fresh credential id. Uses high-resolution clock
/// mixed with a monotonic counter so collisions across threads
/// are astronomically unlikely without pulling in `uuid`.
fn make_db_cred_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("pier-x.db.{nanos:x}{n:x}")
}

/// Save a DB credential under `connection_index`'s profile,
/// using **upsert** semantics: if an existing credential matches
/// the natural key (detection signature, or `(kind, host, port, user)`
/// / `sqlite_path` for SQLite), the existing entry is updated in
/// place and its id is preserved; otherwise a new entry is appended.
///
/// `password` of `Some("")` is treated as "no password"; `None`
/// also means passwordless. When a real password is supplied,
/// storage falls back to `Direct` if the OS keyring silently
/// drops the write (see [`credentials::set_and_verify`]).
///
/// This is the only writer the UI calls when the user clicks
/// "save" in the Add Credential dialog (incl. Adopt & connect),
/// so the upsert is what stops accidental double-saves from
/// surfacing duplicate rows in the Saved Profiles list.
pub fn save_db_credential(
    connection_index: usize,
    input: NewDbCredential,
    password: Option<String>,
) -> Result<DbCredential, DbCredentialError> {
    let mut store = ConnectionStore::load_default()?;
    if connection_index >= store.connections.len() {
        return Err(DbCredentialError::ConnectionIndex(connection_index));
    }

    let databases = &store.connections[connection_index].databases;
    let upsert_idx = find_db_cred_upsert_slot(databases, &input);

    // Favourite is per (connection, kind): clear existing
    // favourite bits for this kind before writing the new one.
    // Done before mutation so the upsert path can later set
    // `favorite = true` on the matched slot in the same pass.
    if input.favorite {
        for (i, other) in store.connections[connection_index]
            .databases
            .iter_mut()
            .enumerate()
        {
            if other.kind == input.kind && Some(i) != upsert_idx {
                other.favorite = false;
            }
        }
    }

    if let Some(idx) = upsert_idx {
        // ── Upsert path ─────────────────────────────────────
        let existing_id = store.connections[connection_index].databases[idx]
            .id
            .clone();
        let prev_keyring_id = match &store.connections[connection_index].databases[idx].password {
            DbPasswordStorage::Keyring { credential_id } => Some(credential_id.clone()),
            _ => None,
        };
        // Best-effort drop of the old keyring entry before
        // writing the new password under the *existing* cred id.
        // Mirrors `update_db_credential`'s rotation flow.
        if let Some(prev) = &prev_keyring_id {
            let _ = credentials::delete(prev);
        }
        let password_storage = store_password(&existing_id, password)?;

        // Apply field updates. Empty `label` / `database` /
        // `sqlite_path` from the input do NOT clobber existing
        // non-empty values — Adopt-flow callers sometimes leave
        // these blank when re-saving an already-named entry.
        let prev_source = store.connections[connection_index].databases[idx]
            .source
            .clone();
        let cred = &mut store.connections[connection_index].databases[idx];
        cred.host = input.host;
        cred.port = input.port;
        cred.user = input.user;
        if !input.label.trim().is_empty() {
            cred.label = input.label;
        }
        if let Some(db) = input.database.filter(|s| !s.is_empty()) {
            cred.database = Some(db);
        }
        if let Some(sp) = input.sqlite_path.filter(|s| !s.is_empty()) {
            cred.sqlite_path = Some(sp);
        }
        cred.favorite = input.favorite;
        // Keep an existing detection signature if the new save
        // is Manual — otherwise re-saving a manually edited
        // adopted cred would forget where it came from.
        cred.source = match (&prev_source, &input.source) {
            (DbCredentialSource::Detected { .. }, DbCredentialSource::Manual) => prev_source,
            _ => input.source,
        };
        cred.password = password_storage.clone();

        let result = cred.clone();
        if let Err(err) = store.save_default() {
            // Only roll back the keyring if we actually wrote a
            // *new* entry under this id. If we're still pointing
            // at the same id with the same value, deleting would
            // wipe a row the on-disk YAML still references.
            if prev_keyring_id.as_deref() != Some(existing_id.as_str()) {
                if let DbPasswordStorage::Keyring { credential_id } = &password_storage {
                    let _ = credentials::delete(credential_id);
                }
            }
            return Err(err.into());
        }
        return Ok(result);
    }

    // ── Insert path ─────────────────────────────────────────
    let id = make_db_cred_id();
    let password_storage = store_password(&id, password)?;

    let cred = DbCredential {
        id: id.clone(),
        kind: input.kind,
        label: input.label,
        host: input.host,
        port: input.port,
        user: input.user,
        database: input.database.filter(|s| !s.is_empty()),
        sqlite_path: input.sqlite_path.filter(|s| !s.is_empty()),
        password: password_storage.clone(),
        favorite: input.favorite,
        source: input.source,
    };
    store.connections[connection_index]
        .databases
        .push(cred.clone());
    // Rollback keyring if the YAML save fails — otherwise we'd
    // leave an orphan keyring entry that no code path ever reads.
    if let Err(err) = store.save_default() {
        if let DbPasswordStorage::Keyring { credential_id } = &password_storage {
            // Best-effort: a missing entry during cleanup is fine.
            let _ = credentials::delete(credential_id);
        }
        return Err(err.into());
    }
    Ok(cred)
}

/// Locate the existing slot to upsert into for `save_db_credential`.
/// Returns `Some(idx)` when an existing credential should be
/// updated rather than a new row appended.
///
/// Matching strategy (kind-scoped — different kinds never collide):
/// - SQLite: match on `sqlite_path` (the natural identity).
/// - Detected source with non-empty `signature`: prefer matching an
///   existing credential with the same `Detected` signature.
/// - Otherwise: match on `(host, port, user)`.
///
/// Returns `None` if no existing slot is a duplicate, or if the
/// would-be key has empty fields (which would otherwise collapse
/// every empty-field cred together).
fn find_db_cred_upsert_slot(
    databases: &[DbCredential],
    input: &NewDbCredential,
) -> Option<usize> {
    use crate::ssh::config::DbCredentialSource as Src;

    if matches!(input.kind, DbKind::Sqlite) {
        let path = input.sqlite_path.as_deref().unwrap_or("").trim();
        if path.is_empty() {
            return None;
        }
        return databases
            .iter()
            .position(|c| c.kind == DbKind::Sqlite && c.sqlite_path.as_deref() == Some(path));
    }

    if let Src::Detected { signature } = &input.source {
        let sig = signature.trim();
        if !sig.is_empty() {
            if let Some(idx) = databases.iter().position(|c| {
                c.kind == input.kind
                    && matches!(&c.source, Src::Detected { signature: s } if s == sig)
            }) {
                return Some(idx);
            }
        }
    }

    let host = input.host.trim();
    let port = input.port;
    let user = input.user.trim();
    if host.is_empty() {
        // Don't collapse empty-host inputs — they'd all match each
        // other, which is the same bug we're trying to fix.
        return None;
    }
    databases.iter().position(|c| {
        c.kind == input.kind
            && c.host.trim() == host
            && c.port == port
            && c.user.trim() == user
    })
}

/// Mutate an existing credential in-place. Unknown fields in
/// the patch are left alone.
pub fn update_db_credential(
    connection_index: usize,
    credential_id: &str,
    patch: DbCredentialPatch,
    new_password: Option<Option<String>>,
) -> Result<DbCredential, DbCredentialError> {
    let mut store = ConnectionStore::load_default()?;
    if connection_index >= store.connections.len() {
        return Err(DbCredentialError::ConnectionIndex(connection_index));
    }
    let idx = store.connections[connection_index]
        .databases
        .iter()
        .position(|c| c.id == credential_id)
        .ok_or_else(|| DbCredentialError::NotFound(credential_id.to_string()))?;

    // Apply patch to a mutable reference.
    {
        let c = &mut store.connections[connection_index].databases[idx];
        if let Some(v) = patch.label {
            c.label = v;
        }
        if let Some(v) = patch.host {
            c.host = v;
        }
        if let Some(v) = patch.port {
            c.port = v;
        }
        if let Some(v) = patch.user {
            c.user = v;
        }
        if let Some(v) = patch.database {
            c.database = v.filter(|s| !s.is_empty());
        }
        if let Some(v) = patch.sqlite_path {
            c.sqlite_path = v.filter(|s| !s.is_empty());
        }
    }

    // If the favourite bit flips on, clear others of the same kind.
    if let Some(fav) = patch.favorite {
        let kind = store.connections[connection_index].databases[idx].kind;
        if fav {
            for (i, other) in store.connections[connection_index]
                .databases
                .iter_mut()
                .enumerate()
            {
                if other.kind == kind {
                    other.favorite = i == idx;
                }
            }
        } else {
            store.connections[connection_index].databases[idx].favorite = false;
        }
    }

    // Rotate the password only when explicitly requested.
    // `Some(Some(pw))` sets new, `Some(None)` clears to
    // passwordless, `None` leaves alone.
    let rotated_password = new_password.is_some();
    if let Some(new) = new_password {
        let existing_id = match &store.connections[connection_index].databases[idx].password {
            DbPasswordStorage::Keyring { credential_id } => Some(credential_id.clone()),
            _ => None,
        };
        if let Some(prev_id) = &existing_id {
            // Best-effort delete; a missing entry is fine.
            let _ = credentials::delete(prev_id);
        }
        let storage_id = existing_id.unwrap_or_else(|| {
            store.connections[connection_index].databases[idx]
                .id
                .clone()
        });
        let storage = store_password(&storage_id, new)?;
        store.connections[connection_index].databases[idx].password = storage;
    }

    let result = store.connections[connection_index].databases[idx].clone();
    // Rollback a brand-new keyring entry if the YAML save fails.
    // We only delete when the rotation created a *new* keyring id
    // — otherwise we'd wipe a keyring row that the prior saved
    // state still references.
    if let Err(err) = store.save_default() {
        if rotated_password {
            if let DbPasswordStorage::Keyring { credential_id } = &result.password {
                let _ = credentials::delete(credential_id);
            }
        }
        return Err(err.into());
    }
    Ok(result)
}

/// Remove a credential and drop its keyring entry if any.
pub fn delete_db_credential(
    connection_index: usize,
    credential_id: &str,
) -> Result<(), DbCredentialError> {
    let mut store = ConnectionStore::load_default()?;
    if connection_index >= store.connections.len() {
        return Err(DbCredentialError::ConnectionIndex(connection_index));
    }
    let databases = &mut store.connections[connection_index].databases;
    let idx = databases
        .iter()
        .position(|c| c.id == credential_id)
        .ok_or_else(|| DbCredentialError::NotFound(credential_id.to_string()))?;
    let removed = databases.remove(idx);
    if let DbPasswordStorage::Keyring { credential_id } = &removed.password {
        // Best-effort; a missing keyring entry is fine.
        let _ = credentials::delete(credential_id);
    }
    // Drop any in-memory Direct plaintext this credential was
    // relying on so we don't leak it across re-adds of the same id.
    cache_forget(&removed.id);
    store.save_default()?;
    Ok(())
}

/// Load a credential plus its password (resolved from keyring
/// when applicable).
pub fn resolve_db_credential(
    connection_index: usize,
    credential_id: &str,
) -> Result<ResolvedDbCredential, DbCredentialError> {
    let store = ConnectionStore::load_default()?;
    let conn = store
        .connections
        .get(connection_index)
        .ok_or(DbCredentialError::ConnectionIndex(connection_index))?;
    let cred = conn
        .databases
        .iter()
        .find(|c| c.id == credential_id)
        .cloned()
        .ok_or_else(|| DbCredentialError::NotFound(credential_id.to_string()))?;

    let password = match &cred.password {
        DbPasswordStorage::Keyring { credential_id } => credentials::get(credential_id)?,
        DbPasswordStorage::Direct { password } => {
            // `Direct.password` is `#[serde(skip)]`, so a freshly
            // loaded store always has an empty string here. Fall
            // back to the process-local plaintext cache that
            // `store_password` mirrored into when the keyring
            // silently dropped the write. Without this the panel
            // would silently send a missing-AUTH request and get
            // a cryptic `NOAUTH` / `Access denied` from the server.
            if !password.is_empty() {
                Some(password.clone())
            } else {
                cache_get(&cred.id)
            }
        }
        DbPasswordStorage::None => None,
    };
    Ok(ResolvedDbCredential {
        credential: cred,
        password,
    })
}

/// Lower `password` into a [`DbPasswordStorage`] variant.
/// `None` / `Some("")` maps to `None`. Real passwords try
/// keyring first; fall back to `Direct` on silent-drop.
fn store_password(
    cred_id: &str,
    password: Option<String>,
) -> Result<DbPasswordStorage, DbCredentialError> {
    let Some(pw) = password.filter(|s| !s.is_empty()) else {
        // Caller is rotating to passwordless — make sure any stale
        // plaintext we had cached for this id is gone too.
        cache_forget(cred_id);
        return Ok(DbPasswordStorage::None);
    };
    // Try keyring. `set_and_verify` returns `Ok(false)` when
    // the backend silently dropped the write.
    match credentials::set_and_verify(cred_id, &pw) {
        Ok(true) => {
            cache_forget(cred_id);
            Ok(DbPasswordStorage::Keyring {
                credential_id: cred_id.to_string(),
            })
        }
        Ok(false) => {
            log::warn!(
                "keyring unavailable for db credential {cred_id}, using in-memory Direct fallback"
            );
            // Mirror the plaintext into the process-local cache so
            // that `resolve_db_credential` can return it even after
            // `save_default()` round-trips the (serde-skipped)
            // Direct field through YAML.
            cache_put(cred_id, pw.clone());
            Ok(DbPasswordStorage::Direct { password: pw })
        }
        Err(e) => Err(DbCredentialError::Credential(e)),
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

    #[test]
    fn reorder_with_groups_permutes_and_assigns_groups() {
        let mut store = ConnectionStore::new();
        store.add(make_config("a", "id-a"));
        store.add(make_config("b", "id-b"));
        store.add(make_config("c", "id-c"));
        // Move c to front, b to end, and tag a/c as "prod" while b stays ungrouped.
        let order = vec![2usize, 0, 1];
        let groups = vec![Some("prod".to_string()), Some("prod".to_string()), None];
        store
            .reorder_with_groups(&order, &groups)
            .expect("reorder ok");
        assert_eq!(store.connections[0].name, "c");
        assert_eq!(store.connections[1].name, "a");
        assert_eq!(store.connections[2].name, "b");
        assert_eq!(store.connections[0].group.as_deref(), Some("prod"));
        assert_eq!(store.connections[1].group.as_deref(), Some("prod"));
        assert_eq!(store.connections[2].group, None);
    }

    #[test]
    fn reorder_rejects_non_permutation() {
        let mut store = ConnectionStore::new();
        store.add(make_config("a", "id-a"));
        store.add(make_config("b", "id-b"));
        // Duplicate index is not a valid permutation.
        let err = store
            .reorder_with_groups(&[0, 0], &[None, None])
            .expect_err("duplicate index must be rejected");
        assert!(format!("{err}").contains("permutation"));
    }

    #[test]
    fn rename_group_updates_matching_members_only() {
        let mut store = ConnectionStore::new();
        let mut a = make_config("a", "id-a");
        a.group = Some("old".into());
        let mut b = make_config("b", "id-b");
        b.group = Some("old".into());
        let mut c = make_config("c", "id-c");
        c.group = Some("keep".into());
        store.add(a);
        store.add(b);
        store.add(c);
        let touched = store.rename_group("old", Some("new"));
        assert_eq!(touched, 2);
        assert_eq!(store.connections[0].group.as_deref(), Some("new"));
        assert_eq!(store.connections[1].group.as_deref(), Some("new"));
        assert_eq!(store.connections[2].group.as_deref(), Some("keep"));
        // Renaming to empty / None strips the group.
        let touched = store.rename_group("new", None);
        assert_eq!(touched, 2);
        assert_eq!(store.connections[0].group, None);
        assert_eq!(store.connections[1].group, None);
    }

    /// Build a fully-populated `DbCredential` for upsert-helper tests.
    fn cred(
        id: &str,
        kind: DbKind,
        host: &str,
        port: u16,
        user: &str,
        source: DbCredentialSource,
    ) -> DbCredential {
        DbCredential {
            id: id.into(),
            kind,
            label: id.into(),
            host: host.into(),
            port,
            user: user.into(),
            database: None,
            sqlite_path: None,
            password: DbPasswordStorage::None,
            favorite: false,
            source,
        }
    }

    fn input(
        kind: DbKind,
        host: &str,
        port: u16,
        user: &str,
        source: DbCredentialSource,
    ) -> NewDbCredential {
        NewDbCredential {
            kind,
            label: "from-input".into(),
            host: host.into(),
            port,
            user: user.into(),
            database: None,
            sqlite_path: None,
            favorite: false,
            source,
        }
    }

    #[test]
    fn upsert_slot_matches_host_port_user_within_same_kind() {
        let dbs = vec![
            cred(
                "a",
                DbKind::Mysql,
                "127.0.0.1",
                3306,
                "root",
                DbCredentialSource::Manual,
            ),
            cred(
                "b",
                DbKind::Postgres,
                "127.0.0.1",
                3306,
                "root",
                DbCredentialSource::Manual,
            ),
        ];
        let i = input(
            DbKind::Mysql,
            "127.0.0.1",
            3306,
            "root",
            DbCredentialSource::Manual,
        );
        // Matches the Mysql row, not the Postgres one with the
        // accidentally identical host/port/user.
        assert_eq!(find_db_cred_upsert_slot(&dbs, &i), Some(0));
    }

    #[test]
    fn upsert_slot_differs_on_user_or_port() {
        let dbs = vec![cred(
            "a",
            DbKind::Mysql,
            "127.0.0.1",
            3306,
            "root",
            DbCredentialSource::Manual,
        )];
        let other_user = input(
            DbKind::Mysql,
            "127.0.0.1",
            3306,
            "readonly",
            DbCredentialSource::Manual,
        );
        let other_port = input(
            DbKind::Mysql,
            "127.0.0.1",
            3307,
            "root",
            DbCredentialSource::Manual,
        );
        assert_eq!(find_db_cred_upsert_slot(&dbs, &other_user), None);
        assert_eq!(find_db_cred_upsert_slot(&dbs, &other_port), None);
    }

    #[test]
    fn upsert_slot_prefers_signature_match_when_detected() {
        // Two adopted creds at the same host/port: distinct
        // detection signatures should be treated as separate.
        let dbs = vec![
            cred(
                "a",
                DbKind::Mysql,
                "127.0.0.1",
                3306,
                "root",
                DbCredentialSource::Detected {
                    signature: "docker://aaa/127.0.0.1:3306".into(),
                },
            ),
            cred(
                "b",
                DbKind::Mysql,
                "127.0.0.1",
                3306,
                "root",
                DbCredentialSource::Detected {
                    signature: "docker://bbb/127.0.0.1:3306".into(),
                },
            ),
        ];
        let i = input(
            DbKind::Mysql,
            "127.0.0.1",
            3306,
            "root",
            DbCredentialSource::Detected {
                signature: "docker://bbb/127.0.0.1:3306".into(),
            },
        );
        assert_eq!(find_db_cred_upsert_slot(&dbs, &i), Some(1));
    }

    #[test]
    fn upsert_slot_falls_back_to_host_port_when_signature_unmatched() {
        // Detected-source input with a brand-new signature still
        // collapses onto an existing same-host/port/user entry.
        // Matches the Adopt-then-double-click case the user hit.
        let dbs = vec![cred(
            "a",
            DbKind::Mysql,
            "127.0.0.1",
            3306,
            "root",
            DbCredentialSource::Manual,
        )];
        let i = input(
            DbKind::Mysql,
            "127.0.0.1",
            3306,
            "root",
            DbCredentialSource::Detected {
                signature: "docker://abc/127.0.0.1:3306".into(),
            },
        );
        assert_eq!(find_db_cred_upsert_slot(&dbs, &i), Some(0));
    }

    #[test]
    fn upsert_slot_sqlite_uses_path() {
        let mut a = cred(
            "a",
            DbKind::Sqlite,
            "",
            0,
            "",
            DbCredentialSource::Manual,
        );
        a.sqlite_path = Some("/srv/app.db".into());
        let mut b = cred(
            "b",
            DbKind::Sqlite,
            "",
            0,
            "",
            DbCredentialSource::Manual,
        );
        b.sqlite_path = Some("/srv/other.db".into());
        let dbs = vec![a, b];
        let mut i = input(
            DbKind::Sqlite,
            "",
            0,
            "",
            DbCredentialSource::Manual,
        );
        i.sqlite_path = Some("/srv/other.db".into());
        assert_eq!(find_db_cred_upsert_slot(&dbs, &i), Some(1));
    }

    #[test]
    fn upsert_slot_returns_none_for_blank_host() {
        // Empty-host input must not collide with another empty-host
        // entry — that's exactly the collapse the helper avoids.
        let dbs = vec![cred(
            "a",
            DbKind::Mysql,
            "",
            0,
            "",
            DbCredentialSource::Manual,
        )];
        let i = input(DbKind::Mysql, "", 0, "", DbCredentialSource::Manual);
        assert_eq!(find_db_cred_upsert_slot(&dbs, &i), None);
    }

    #[test]
    fn group_field_round_trips_through_json() {
        let path = fresh_tmp("group-rt");
        let mut store = ConnectionStore::new();
        let mut a = make_config("a", "id-a");
        a.group = Some("prod".into());
        store.add(a);
        store.add(make_config("b", "id-b")); // group = None
        store.save_to_path(&path).expect("save");
        let loaded = ConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded.connections[0].group.as_deref(), Some("prod"));
        assert_eq!(loaded.connections[1].group, None);
        let _ = fs::remove_file(&path);
    }
}
