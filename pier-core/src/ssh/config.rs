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

/// Addressing + auth for one SSH connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConfig {
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
}

fn default_connect_timeout() -> u64 {
    10
}

impl SshConfig {
    /// Create a config with sensible defaults. `name` doubles as
    /// the display label if you don't override it.
    pub fn new(name: impl Into<String>, host: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            host: host.into(),
            port: 22,
            user: user.into(),
            auth: AuthMethod::KeychainPassword {
                credential_id: String::new(),
            },
            connect_timeout_secs: default_connect_timeout(),
            tags: Vec::new(),
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
    /// In-memory password. Tests only. NOT serialized to disk
    /// because of `skip_serializing_if = "always_true"`.
    #[serde(skip)]
    InMemoryPassword {
        /// The actual password bytes. Never persisted.
        password: String,
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
            Self::InMemoryPassword { password } => !password.is_empty(),
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
        // The C++ connection store writes the on-disk JSON by
        // hand, so we need to pin Rust's serde output for the
        // Agent variant. Any refactor that breaks this string
        // will silently break the C++ round-trip.
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
        };
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: SshConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, original);
    }

    #[test]
    fn in_memory_password_is_never_serialized() {
        let c = SshConfig {
            name: "tmp".into(),
            host: "h".into(),
            port: 22,
            user: "u".into(),
            auth: AuthMethod::InMemoryPassword {
                password: "hunter2".into(),
            },
            connect_timeout_secs: 5,
            tags: vec![],
        };
        // InMemoryPassword is marked #[serde(skip)] — serialization
        // should fail or produce a placeholder rather than include
        // the secret.
        let json_result = serde_json::to_string(&c);
        if let Ok(json) = &json_result {
            assert!(
                !json.contains("hunter2"),
                "password leaked into serialized form: {json}",
            );
        }
    }
}
