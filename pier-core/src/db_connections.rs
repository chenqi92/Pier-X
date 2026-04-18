//! Persisted database connection list.
//!
//! Parallel to [`crate::connections`] (SSH). Stores the sidebar list
//! of saved database connections as a thin `serde_json` wrapper
//! around `Vec<DbConnection>` plus a schema version, with atomic
//! load/save through [`crate::paths::db_connections_file`].
//!
//! ## What does NOT live here
//!
//! Passwords. [`DbConnection::credential_id`] only ever holds an
//! opaque keychain key — the actual password is stored in the OS
//! keychain via [`crate::credentials`] under the service
//! `com.kkape.pier-x` and looked up at connect time. The persisted
//! JSON therefore contains nothing the user couldn't safely sync
//! across machines.
//!
//! The convention for the `credential_id` value is
//! `pier-x.db.{engine}.{name}`, mirroring the SSH side which uses
//! `pier-x.password.{name}`. Using a distinct `pier-x.db.*` prefix
//! keeps the two keychain namespaces from colliding.
//!
//! ## File format
//!
//! ```json
//! {
//!   "version": 1,
//!   "connections": [
//!     {
//!       "name": "prod",
//!       "engine": "mysql",
//!       "host": "127.0.0.1",
//!       "port": 3306,
//!       "user": "root",
//!       "database": "app",
//!       "credential_id": "pier-x.db.mysql.prod"
//!     }
//!   ]
//! }
//! ```
//!
//! ## Atomicity
//!
//! [`DbConnectionStore::save_to_path`] writes to a sibling
//! `.db-connections.json.tmp` file then atomically renames it, the
//! same pattern used by [`crate::connections::ConnectionStore`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::paths;

/// Current on-disk schema version. Bumped on any breaking change
/// to the JSON shape.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Errors that can occur loading or saving the database connections
/// file.
#[derive(Debug, thiserror::Error)]
pub enum DbConnectionStoreError {
    /// I/O error reading or writing the file.
    #[error("db connections store I/O: {0}")]
    Io(#[from] io::Error),

    /// JSON parse error — the file exists but is malformed.
    #[error("db connections store JSON: {0}")]
    Json(#[from] serde_json::Error),

    /// The file's `version` field is from the future and we don't
    /// know how to migrate. Caller should typically rename the file
    /// aside and start fresh rather than overwrite it.
    #[error("db connections store version {found} > supported {supported}")]
    FutureVersion {
        /// The version stamped on the file we just read.
        found: u32,
        /// The highest version this build of pier-core understands.
        supported: u32,
    },

    /// `paths::db_connections_file()` returned None — no usable
    /// home directory.
    #[error("no usable application data directory")]
    NoDataDir,
}

/// Database engine this connection talks to. Phase A supports
/// MySQL and PostgreSQL only — Redis and SQLite land in later
/// phases under their own variants.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DbEngine {
    /// MySQL / MariaDB via `mysql_async`.
    Mysql,
    /// PostgreSQL via `tokio-postgres`.
    Postgres,
}

impl DbEngine {
    /// Short stable string used in keychain IDs (`pier-x.db.{engine}.{name}`)
    /// and anywhere else an engine prefix is needed.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mysql => "mysql",
            Self::Postgres => "postgres",
        }
    }

    /// Default TCP port for this engine. Used as the form's initial
    /// value when the user hasn't typed one yet.
    pub fn default_port(self) -> u16 {
        match self {
            Self::Mysql => 3306,
            Self::Postgres => 5432,
        }
    }
}

/// One saved database connection. Name uniqueness is not enforced
/// in the store — the UI layer deduplicates when minting keychain
/// IDs so a collision on `pier-x.db.{engine}.{name}` can't happen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DbConnection {
    /// Human-readable label, shown in the connection dropdown.
    pub name: String,
    /// Which backend engine (`MysqlClient` vs `PostgresClient`).
    pub engine: DbEngine,
    /// Hostname or IP the engine listens on. Usually `127.0.0.1`
    /// when going through an SSH tunnel.
    pub host: String,
    /// TCP port. See [`DbEngine::default_port`] for defaults.
    pub port: u16,
    /// Database user.
    pub user: String,
    /// Default database to connect to. `None` = connect without
    /// selecting one (MySQL default), or use the PG role's default.
    #[serde(default)]
    pub database: Option<String>,
    /// Keychain key holding the password. `None` = connect with
    /// empty password (rare but legal). Format:
    /// `pier-x.db.{engine}.{name}`.
    #[serde(default)]
    pub credential_id: Option<String>,
}

impl DbConnection {
    /// Mint the keychain id for this connection's password. Used by
    /// the UI layer when saving a new connection so the id matches
    /// the format the rest of the app expects.
    pub fn credential_id_for(engine: DbEngine, name: &str) -> String {
        format!("pier-x.db.{}.{}", engine.as_str(), name.trim())
    }
}

/// Top-level on-disk representation. Holds a versioned database
/// connections list. New fields are appended in future versions
/// behind `#[serde(default)]` so older files load forward.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DbConnectionStore {
    /// Schema version stamped on the file. Always written as
    /// [`CURRENT_SCHEMA_VERSION`]; on read, used to dispatch
    /// version-specific migrations.
    pub version: u32,
    /// The actual connections list, in display order.
    #[serde(default)]
    pub connections: Vec<DbConnection>,
}

impl Default for DbConnectionStore {
    fn default() -> Self {
        Self {
            version: CURRENT_SCHEMA_VERSION,
            connections: Vec::new(),
        }
    }
}

impl DbConnectionStore {
    /// Build an empty store stamped at the current schema version.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the store from the standard location resolved by
    /// [`crate::paths::db_connections_file`]. Returns the default
    /// (empty) store if the file does not yet exist.
    pub fn load_default() -> Result<Self, DbConnectionStoreError> {
        let path = paths::db_connections_file().ok_or(DbConnectionStoreError::NoDataDir)?;
        Self::load_from_path(&path)
    }

    /// Load from an explicit path. Used by tests; production
    /// callers should use [`Self::load_default`].
    ///
    /// Missing file → `Ok(Self::default())`. Malformed file →
    /// [`DbConnectionStoreError::Json`]. Future schema →
    /// [`DbConnectionStoreError::FutureVersion`].
    pub fn load_from_path(path: &Path) -> Result<Self, DbConnectionStoreError> {
        match fs::read(path) {
            Ok(bytes) => {
                let store: Self = serde_json::from_slice(&bytes)?;
                if store.version > CURRENT_SCHEMA_VERSION {
                    return Err(DbConnectionStoreError::FutureVersion {
                        found: store.version,
                        supported: CURRENT_SCHEMA_VERSION,
                    });
                }
                Ok(store)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persist the store to the standard location. Creates the
    /// containing directory if it doesn't yet exist.
    pub fn save_default(&self) -> Result<(), DbConnectionStoreError> {
        let path = paths::db_connections_file().ok_or(DbConnectionStoreError::NoDataDir)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        self.save_to_path(&path)
    }

    /// Persist to an explicit path, atomically: write to a sibling
    /// `.tmp` then rename. The version field is forced to
    /// [`CURRENT_SCHEMA_VERSION`] before serialization.
    pub fn save_to_path(&self, path: &Path) -> Result<(), DbConnectionStoreError> {
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
    pub fn add(&mut self, connection: DbConnection) {
        self.connections.push(connection);
    }

    /// Replace the connection at `index`. Returns the old value, or
    /// `None` if the index is out of range.
    pub fn replace(&mut self, index: usize, connection: DbConnection) -> Option<DbConnection> {
        if index < self.connections.len() {
            Some(std::mem::replace(&mut self.connections[index], connection))
        } else {
            None
        }
    }

    /// Remove the connection at `index`. Out-of-range indices are
    /// silently ignored so callers can treat removal as idempotent.
    pub fn remove(&mut self, index: usize) -> Option<DbConnection> {
        if index < self.connections.len() {
            Some(self.connections.remove(index))
        } else {
            None
        }
    }
}

/// Compute the temp-file sibling path used by atomic save. `.tmp`
/// is appended to the file name (preserving the parent directory)
/// so the rename is single-directory-atomic.
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
    use std::env::temp_dir;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fresh_tmp(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        temp_dir().join(format!("pier-x-test-dbconn-{label}-{pid}-{n}.json"))
    }

    fn sample(name: &str, engine: DbEngine) -> DbConnection {
        DbConnection {
            name: name.into(),
            engine,
            host: "127.0.0.1".into(),
            port: engine.default_port(),
            user: "root".into(),
            database: Some("app".into()),
            credential_id: Some(DbConnection::credential_id_for(engine, name)),
        }
    }

    #[test]
    fn empty_store_round_trips() {
        let path = fresh_tmp("empty");
        let store = DbConnectionStore::new();
        store.save_to_path(&path).expect("save");
        let loaded = DbConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded, store);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn nonexistent_file_returns_default() {
        let path = fresh_tmp("missing");
        assert!(!path.exists());
        let loaded = DbConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded, DbConnectionStore::default());
    }

    #[test]
    fn mixed_engine_round_trip() {
        let path = fresh_tmp("mixed");
        let mut store = DbConnectionStore::new();
        store.add(sample("prod-mysql", DbEngine::Mysql));
        store.add(sample("prod-pg", DbEngine::Postgres));
        store.save_to_path(&path).expect("save");

        let loaded = DbConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded.version, CURRENT_SCHEMA_VERSION);
        assert_eq!(loaded.connections.len(), 2);
        assert_eq!(loaded.connections[0].engine, DbEngine::Mysql);
        assert_eq!(loaded.connections[0].port, 3306);
        assert_eq!(loaded.connections[1].engine, DbEngine::Postgres);
        assert_eq!(loaded.connections[1].port, 5432);
        assert_eq!(
            loaded.connections[0].credential_id.as_deref(),
            Some("pier-x.db.mysql.prod-mysql")
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn connection_without_password_round_trips() {
        let path = fresh_tmp("nopass");
        let mut store = DbConnectionStore::new();
        store.add(DbConnection {
            name: "local".into(),
            engine: DbEngine::Mysql,
            host: "127.0.0.1".into(),
            port: 3306,
            user: "root".into(),
            database: None,
            credential_id: None,
        });
        store.save_to_path(&path).expect("save");

        let loaded = DbConnectionStore::load_from_path(&path).expect("load");
        assert_eq!(loaded.connections[0].credential_id, None);
        assert_eq!(loaded.connections[0].database, None);
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
        let err = DbConnectionStore::load_from_path(&path).expect_err("future-version rejected");
        match err {
            DbConnectionStoreError::FutureVersion { found, supported } => {
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
        fs::write(&path, b"not json").unwrap();
        let err = DbConnectionStore::load_from_path(&path).expect_err("garbage rejected");
        assert!(matches!(err, DbConnectionStoreError::Json(_)));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn engine_serializes_as_lowercase() {
        let encoded = serde_json::to_string(&DbEngine::Mysql).unwrap();
        assert_eq!(encoded, "\"mysql\"");
        let encoded = serde_json::to_string(&DbEngine::Postgres).unwrap();
        assert_eq!(encoded, "\"postgres\"");
    }

    #[test]
    fn credential_id_format_is_stable() {
        assert_eq!(
            DbConnection::credential_id_for(DbEngine::Mysql, "prod"),
            "pier-x.db.mysql.prod"
        );
        assert_eq!(
            DbConnection::credential_id_for(DbEngine::Postgres, "analytics"),
            "pier-x.db.postgres.analytics"
        );
    }
}
