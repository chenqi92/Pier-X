//! SSH connection configuration — plain data, no I/O, no secrets.
//!
//! [`SshConfig`] holds the addressing + authentication information
//! needed to establish one SSH session. It is deliberately
//! `Serialize + Deserialize` so it can round-trip through the
//! connections JSON file that M3b's connection manager will save
//! to `~/.config/pier-x/connections.json`.
//!
//! Secrets (passwords, key passphrases) are NOT stored in
//! [`SshConfig`]. The field that *would* hold a password is a
//! stable *credential handle* — an opaque string that M3b's
//! keyring integration will use to look up the actual secret in
//! the OS keychain. That way the connections file can be synced
//! across machines or shared without leaking credentials, while
//! the real bytes stay in Keychain / DPAPI / Secret Service.

use serde::{Deserialize, Serialize};

/// Which protocol a saved connection speaks. `ssh` (the default for every
/// pre-existing entry) opens a terminal; `rdp` / `vnc` open a remote-desktop
/// tab handled by [`crate::remote_desktop`]. For `rdp` / `vnc` only the
/// addressing + password auth fields are meaningful — `databases`,
/// `egress_id`, `auto_elevate`, key/agent auth are SSH-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionProtocol {
    /// SSH terminal (default).
    #[default]
    Ssh,
    /// Microsoft Remote Desktop.
    Rdp,
    /// RFB / VNC (incl. macOS Screen Sharing).
    Vnc,
}

/// Addressing + auth for one SSH connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConfig {
    /// Connection protocol. Defaults to [`ConnectionProtocol::Ssh`] so saved
    /// connections written before remote-desktop support deserialize cleanly.
    #[serde(default)]
    pub protocol: ConnectionProtocol,
    /// RDP only: optional Windows domain / NetBIOS name. Ignored for
    /// SSH and VNC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Human-readable label, shown in the sidebar. Not part of the
    /// SSH protocol — purely UI.
    pub name: String,
    /// Hostname or IP address. No scheme / port.
    pub host: String,
    /// TCP port. Defaults to 22 via [`SshConfig::new`].
    pub port: u16,
    /// Remote user name.
    pub user: String,
    /// How to prove we are `user`.
    pub auth: AuthMethod,
    /// TCP connect timeout, in seconds. `0` means "OS default".
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_secs: u64,
    /// Optional free-form tags, used by the sidebar grouping UI
    /// in M3b+. Ignored by the SSH layer itself.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional group label used by the sidebar to cluster saved
    /// connections. `None` or empty means the connection lives in
    /// the implicit "default" group. Ignored by the SSH layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Optional environment tag (e.g. `prod` / `staging` / `dev` /
    /// `local`). The host lists / bus view / panel headers render
    /// a colored chip when set so a prod box is unmistakable from
    /// staging at a glance. Free-form — UI styles known names but
    /// passes through anything else with neutral styling. Ignored
    /// by the SSH layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_tag: Option<String>,
    /// Database connections remembered for this SSH profile.
    /// Passwords live in the OS keyring via `DbPasswordStorage`.
    /// Ignored by the SSH layer — consumed by the right-side DB
    /// panels to seed forms and auto-open tunnels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub databases: Vec<DbCredential>,
    /// Optional reference to a `crate::egress::EgressProfile::id` in
    /// the same `ConnectionStore`. `None` = direct connection. The
    /// SSH layer has not yet wired this through (stage B); today
    /// only persistence and round-trip are guaranteed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_id: Option<String>,
    /// When true and an elevation password is in the keychain for
    /// this `(user, host, port)` triple, the SSH terminal session
    /// runs `sudo -S -p '' -i bash` immediately after the shell
    /// prompt appears, so the user lands in a root shell without
    /// having to type `sudo -i` themselves. Requires that the user
    /// has previously saved a sudo password (via the prompt's
    /// "Remember" checkbox or NewConnectionDialog's "Sudo password"
    /// field). Off by default — connecting as a non-root user is
    /// a common deliberate choice (audit logs, MFA, …) and
    /// silently elevating would surprise that workflow.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_elevate: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn default_connect_timeout() -> u64 {
    10
}

impl SshConfig {
    /// Create a config with sensible defaults. `name` doubles as
    /// the display label if you don't override it.
    pub fn new(name: impl Into<String>, host: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            protocol: ConnectionProtocol::Ssh,
            domain: None,
            name: name.into(),
            host: host.into(),
            port: 22,
            user: user.into(),
            auth: AuthMethod::KeychainPassword {
                credential_id: String::new(),
            },
            connect_timeout_secs: default_connect_timeout(),
            tags: Vec::new(),
            group: None,
            env_tag: None,
            auto_elevate: false,
            databases: Vec::new(),
            egress_id: None,
        }
    }

    /// True if the config has the minimum required fields set.
    /// Used by the UI to enable/disable the "Connect" button.
    pub fn is_valid(&self) -> bool {
        !self.host.trim().is_empty()
            && !self.user.trim().is_empty()
            && self.port > 0
            && self.auth.is_configured()
    }

    /// The `host:port` pair, ready to hand to [`std::net::ToSocketAddrs`].
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// How to prove identity to the remote sshd.
///
/// The variants are ordered from simplest-to-store (keychain
/// reference) to most-flexible (inline in-memory secret used only
/// by tests).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Password stored in the OS keychain under `credential_id`.
    /// The ssh layer does the lookup via [`crate::credentials`]
    /// right before the authentication attempt, so the secret is
    /// only in memory for the duration of the handshake.
    KeychainPassword {
        /// Opaque ID the keyring crate will look up.
        credential_id: String,
    },
    /// OpenSSH private key file on disk. Passphrase, if any, is
    /// stored in the keychain under `passphrase_credential_id`.
    PublicKeyFile {
        /// Absolute path to the private key (e.g. `~/.ssh/id_ed25519`).
        private_key_path: String,
        /// `None` if the key is unencrypted.
        passphrase_credential_id: Option<String>,
    },
    /// Authenticate via the system SSH agent (`SSH_AUTH_SOCK`).
    /// No secret is stored by pier-core at all — the agent holds
    /// the key.
    Agent,
    /// Password stored directly in the connection config file.
    /// Simpler than the keychain path — works even when the OS
    /// keychain is unavailable or has been cleared.
    DirectPassword {
        /// The password in plaintext.
        password: String,
    },
    /// Try several credentialless methods in sequence — mirrors what
    /// OpenSSH's own client does when the user runs plain
    /// `ssh user@host` without an explicit `-i`: agent first, then
    /// the conventional default key files in `~/.ssh/`. Used by the
    /// right-side Server Monitor / SFTP / Docker panels when the
    /// terminal-side ssh child authenticated without ever showing a
    /// password prompt (public-key + no saved connection), so that
    /// we can reach the same host without asking the user for a
    /// credential we don't have.
    ///
    /// Each inner method is tried on a single session; the first
    /// successful one wins. If everything rejects, the resulting
    /// `AuthRejected` error lists each method that was attempted.
    Auto,
    /// Like [`AuthMethod::Auto`], but additionally consumes any
    /// credentials we managed to capture from the terminal side.
    /// Used when the watcher inferred plain `auto` (the user typed
    /// `ssh user@host` with no saved profile) but we later harvested
    /// extra signals: a `-i <key>` arg, or the password the user typed
    /// at the OpenSSH prompt.
    ///
    /// Chain order on a single SSH transport (one TCP/kex, N userauth
    /// rounds): agent → explicit key → conventional default identity
    /// files → password → keyboard-interactive (with `password` as the
    /// answer to every prompt — that's what most PAM-backed servers
    /// actually negotiate, and it matches what the user just typed
    /// successfully into their `ssh` child). The first method the
    /// server accepts wins; only if every method we have evidence for
    /// rejects do we surface `AuthRejected`.
    AutoChain {
        /// `-i <path>` from the watcher, if any. Tried after agent
        /// and before the conventional default identity files. None
        /// when the user didn't pass `-i`.
        explicit_key_path: Option<String>,
        /// Captured from the OpenSSH password prompt, if any. Drives
        /// both the plain password method and the keyboard-interactive
        /// fallback.
        password: Option<String>,
        /// Captured from an OpenSSH `Enter passphrase for key`
        /// prompt, if any. Used to decrypt the explicit key and the
        /// conventional default identity files when they are
        /// passphrase-protected on disk and the user just entered
        /// the passphrase in their terminal.
        key_passphrase: Option<String>,
    },
}

impl AuthMethod {
    /// True if the method has whatever it needs to attempt
    /// authentication (path set, credential id set, etc.).
    pub fn is_configured(&self) -> bool {
        match self {
            Self::KeychainPassword { credential_id } => !credential_id.is_empty(),
            Self::PublicKeyFile {
                private_key_path, ..
            } => !private_key_path.is_empty(),
            Self::Agent => true,
            Self::DirectPassword { password } => !password.is_empty(),
            Self::Auto => true,
            // Worst case AutoChain falls back to agent + default
            // identity files, same surface as Auto.
            Self::AutoChain { .. } => true,
        }
    }
}

/// Kind of database a [`DbCredential`] refers to. Maps 1:1 with
/// the four right-side DB panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    /// MySQL / MariaDB / Percona (3306).
    Mysql,
    /// PostgreSQL / TimescaleDB / Citus (5432).
    Postgres,
    /// Redis / Valkey / KeyDB (6379).
    Redis,
    /// Local or remote SQLite `.db` file.
    Sqlite,
    /// Microsoft SQL Server (1433).
    Sqlserver,
    /// InfluxDB time-series (8086). `database` = bucket/db, password
    /// slot carries the 2.x API token or 1.x password.
    Influx,
    /// Oracle Database (1521) via remote `sqlplus`. `database` = service
    /// name / SID.
    Oracle,
    /// Dameng (达梦) (5236) via remote `disql`.
    Dameng,
}

/// How the password for a DB credential is stored.
///
/// `Keyring` is the normal case. `Direct` is the fallback used when the
/// OS keyring is unavailable (same shape as [`AuthMethod::DirectPassword`],
/// which it mirrors deliberately): the plaintext is persisted to the
/// connections file so the credential is remembered across restarts on
/// machines whose keyring silently drops writes. This is `#[serde(default)]`
/// rather than `#[serde(skip)]` so it round-trips; the keyring is always
/// tried first (see `connections::store_password`), so `Direct` — and the
/// plaintext on disk — only appears when there is no secure store to use,
/// exactly like the SSH `DirectPassword` already in the same file. `None`
/// is used for Redis (no AUTH configured) and SQLite (passwordless).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DbPasswordStorage {
    /// Password stored in the OS keychain under `credential_id`.
    Keyring {
        /// Opaque key used by [`crate::credentials`].
        credential_id: String,
    },
    /// Plaintext password persisted in the connections file. Only used as
    /// the keyring-unavailable fallback (the keyring is tried first).
    Direct {
        /// Plaintext password. `#[serde(default)]` keeps old files that
        /// were written while this field was serde-skipped loading cleanly
        /// (missing field → empty string, which then re-prompts once).
        #[serde(default)]
        password: String,
    },
    /// Passwordless authentication (Redis with no AUTH, SQLite).
    None,
}

impl DbPasswordStorage {
    /// True if a password is present (keyring ref or runtime
    /// plaintext). Used by the frontend via `hasPassword`.
    pub fn is_present(&self) -> bool {
        match self {
            Self::Keyring { credential_id } => !credential_id.is_empty(),
            Self::Direct { password } => !password.is_empty(),
            Self::None => false,
        }
    }
}

/// Whether a credential was typed by the user or pre-filled from
/// an auto-detected instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DbCredentialSource {
    /// User typed all the fields.
    Manual,
    /// Started from a detection result; `signature` is the
    /// stable dedupe key emitted by `db_detect`.
    Detected {
        /// Stable signature (e.g. `docker://<containerId>:<hostPort>`).
        signature: String,
    },
}

/// A single remembered database target, attached to an SSH
/// profile. Enough fields to re-open a tunnel and browse without
/// any user typing; passwords resolved lazily at connect time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbCredential {
    /// Stable id — shaped `pier-x.db.<uuid>`. Also used as the
    /// keyring key when `password` is `Keyring`.
    pub id: String,
    /// Which panel (mysql / postgres / redis / sqlite).
    pub kind: DbKind,
    /// User-facing label (e.g. "prod-main", "legacy-5.7").
    pub label: String,
    /// Remote-side host. Typically `127.0.0.1` because the tunnel
    /// forwards a remote-loopback port. For `Sqlite` this is unused.
    #[serde(default)]
    pub host: String,
    /// Remote-side port. Unused for `Sqlite`.
    #[serde(default)]
    pub port: u16,
    /// DB user. Empty for Redis/Sqlite.
    #[serde(default)]
    pub user: String,
    /// Default database / schema / Redis DB index.
    #[serde(default)]
    pub database: Option<String>,
    /// Absolute remote path when `kind == Sqlite`, else `None`.
    #[serde(default)]
    pub sqlite_path: Option<String>,
    /// Password storage strategy for this credential.
    pub password: DbPasswordStorage,
    /// When multiple credentials of the same kind exist on one
    /// profile, the favorite seeds the tab on open.
    #[serde(default)]
    pub favorite: bool,
    /// Whether the user typed this or adopted it from detection.
    #[serde(default = "default_db_cred_source_manual")]
    pub source: DbCredentialSource,
    /// Optional reference to an `crate::egress::EgressProfile::id`.
    /// When set, DB connections to this credential should be made
    /// through the profile (typically via a local TCP forwarder),
    /// not directly to `host:port`. `None` = direct (or via the
    /// owning SSH session's tunnel, which is the legacy path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress_id: Option<String>,
}

fn default_db_cred_source_manual() -> DbCredentialSource {
    DbCredentialSource::Manual
}

impl DbCredential {
    /// Whether the credential has enough fields set to attempt
    /// a connection (panel uses this to enable auto-browse).
    pub fn is_valid(&self) -> bool {
        if self.id.trim().is_empty() {
            return false;
        }
        match self.kind {
            DbKind::Sqlite => !self.sqlite_path.as_deref().unwrap_or("").trim().is_empty(),
            _ => !self.host.trim().is_empty() && self.port > 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_port_22_and_10_second_timeout() {
        let c = SshConfig::new("prod", "db.example.com", "deploy");
        assert_eq!(c.port, 22);
        assert_eq!(c.connect_timeout_secs, 10);
        assert_eq!(c.address(), "db.example.com:22");
    }

    #[test]
    fn empty_host_or_user_is_invalid() {
        let mut c = SshConfig::new("", "", "");
        assert!(!c.is_valid());
        c.host = "h".into();
        assert!(!c.is_valid()); // user still empty
        c.user = "u".into();
        assert!(!c.is_valid()); // keychain id still empty
        c.auth = AuthMethod::Agent;
        assert!(c.is_valid());
    }

    #[test]
    fn agent_variant_serializes_as_kind_agent() {
        // Keep serde output pinned for the Agent variant so any
        // persisted configs or fixtures that rely on this exact
        // shape continue to round-trip cleanly.
        let mut c = SshConfig::new("test", "example.com", "root");
        c.auth = AuthMethod::Agent;
        let json = serde_json::to_value(&c).unwrap();
        let auth = json.get("auth").expect("auth field");
        assert_eq!(
            auth.get("kind").and_then(|v| v.as_str()),
            Some("agent"),
            "Agent variant must serialize as kind: \"agent\"; full auth = {auth}",
        );
    }

    #[test]
    fn round_trips_through_json() {
        let original = SshConfig {
            protocol: ConnectionProtocol::Ssh,
            domain: None,
            name: "prod".into(),
            host: "db.example.com".into(),
            port: 2222,
            user: "deploy".into(),
            auth: AuthMethod::PublicKeyFile {
                private_key_path: "/home/me/.ssh/id_ed25519".into(),
                passphrase_credential_id: Some("prod-key-pass".into()),
            },
            connect_timeout_secs: 30,
            tags: vec!["prod".into(), "eu-west".into()],
            group: Some("prod".into()),
            env_tag: Some("prod".into()),
            databases: Vec::new(),
            egress_id: None,
            auto_elevate: false,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: SshConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, original);
    }

    #[test]
    fn db_credential_keyring_round_trips() {
        // Keyring storage roundtrips cleanly and only exposes
        // the opaque credential_id — never a password.
        let original = DbCredential {
            id: "pier-x.db.abc".into(),
            kind: DbKind::Mysql,
            label: "prod-main".into(),
            host: "127.0.0.1".into(),
            port: 3306,
            user: "root".into(),
            database: Some("shop".into()),
            sqlite_path: None,
            password: DbPasswordStorage::Keyring {
                credential_id: "pier-x.db.abc".into(),
            },
            favorite: true,
            source: DbCredentialSource::Manual,
            egress_id: None,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: DbCredential = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, original);
        // A password literal must never appear anywhere in the
        // serialized form.
        assert!(!json.contains("\"password\":\""));
    }

    #[test]
    fn db_credential_direct_password_round_trips() {
        // `DbPasswordStorage::Direct::password` now persists (the
        // keyring-unavailable fallback) so the credential is remembered
        // across restarts, mirroring the SSH `DirectPassword` already in
        // the same file.
        let original = DbCredential {
            id: "pier-x.db.x".into(),
            kind: DbKind::Redis,
            label: "local".into(),
            host: "127.0.0.1".into(),
            port: 6379,
            user: String::new(),
            database: Some("0".into()),
            sqlite_path: None,
            password: DbPasswordStorage::Direct {
                password: "hunter2".into(),
            },
            favorite: false,
            source: DbCredentialSource::Manual,
            egress_id: None,
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: DbCredential = serde_json::from_str(&json).expect("deserialize");
        match parsed.password {
            DbPasswordStorage::Direct { password } => assert_eq!(password, "hunter2"),
            other => panic!("expected Direct, got {other:?}"),
        }
    }

    #[test]
    fn db_credential_direct_missing_password_field_defaults_empty() {
        // Files written while `Direct::password` was `#[serde(skip)]` have
        // no `password` key; `#[serde(default)]` must load them as empty
        // (which re-prompts once) rather than failing the whole store.
        let json = r#"{
            "id": "pier-x.db.legacy",
            "kind": "redis",
            "label": "old",
            "host": "127.0.0.1",
            "port": 6379,
            "user": "",
            "database": "0",
            "sqlite_path": null,
            "password": { "kind": "direct" },
            "favorite": false,
            "source": { "kind": "manual" },
            "egress_id": null
        }"#;
        let parsed: DbCredential = serde_json::from_str(json).expect("deserialize legacy");
        match parsed.password {
            DbPasswordStorage::Direct { password } => assert_eq!(password, ""),
            other => panic!("expected Direct, got {other:?}"),
        }
    }

    #[test]
    fn db_credential_sqlite_uses_path_not_port() {
        let c = DbCredential {
            id: "pier-x.db.s".into(),
            kind: DbKind::Sqlite,
            label: "app.db".into(),
            host: String::new(),
            port: 0,
            user: String::new(),
            database: None,
            sqlite_path: Some("/srv/app.db".into()),
            password: DbPasswordStorage::None,
            favorite: false,
            source: DbCredentialSource::Detected {
                signature: "file:///srv/app.db".into(),
            },
            egress_id: None,
        };
        assert!(c.is_valid(), "sqlite credential with path must be valid");
        let mut bad = c.clone();
        bad.sqlite_path = None;
        assert!(
            !bad.is_valid(),
            "sqlite credential without path must be invalid"
        );
    }

    #[test]
    fn ssh_config_databases_round_trips() {
        let mut cfg = SshConfig::new("prod", "db.example.com", "deploy");
        cfg.databases.push(DbCredential {
            id: "pier-x.db.1".into(),
            kind: DbKind::Postgres,
            label: "primary".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            user: "postgres".into(),
            database: Some("app".into()),
            sqlite_path: None,
            password: DbPasswordStorage::Keyring {
                credential_id: "pier-x.db.1".into(),
            },
            favorite: true,
            source: DbCredentialSource::Manual,
            egress_id: None,
        });
        let json = serde_json::to_string(&cfg).expect("serialize");
        let parsed: SshConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn ssh_config_old_json_without_databases_loads_with_empty_vec() {
        // Forward-compat: an SshConfig written by an older
        // pier-x build that predates `databases` must still load
        // cleanly into the new struct.
        let old = serde_json::json!({
            "name": "legacy",
            "host": "h",
            "port": 22,
            "user": "u",
            "auth": { "kind": "agent" },
            "connect_timeout_secs": 10,
        });
        let parsed: SshConfig = serde_json::from_value(old).expect("backwards-compat load");
        assert!(parsed.databases.is_empty());
    }

    #[test]
    fn direct_password_round_trips() {
        let c = SshConfig {
            protocol: ConnectionProtocol::Ssh,
            domain: None,
            name: "tmp".into(),
            host: "h".into(),
            port: 22,
            user: "u".into(),
            auth: AuthMethod::DirectPassword {
                password: "hunter2".into(),
            },
            connect_timeout_secs: 5,
            tags: vec![],
            group: None,
            env_tag: None,
            databases: Vec::new(),
            egress_id: None,
            auto_elevate: false,
        };
        let json = serde_json::to_string(&c).expect("serialize");
        let parsed: SshConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            parsed.auth,
            AuthMethod::DirectPassword {
                password: "hunter2".into(),
            }
        );
    }
}
