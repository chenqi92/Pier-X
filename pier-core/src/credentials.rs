//! Cross-platform credential storage via the OS keyring.
//!
//! Backends:
//! - macOS:  Keychain
//! - Windows: Credential Manager (DPAPI)
//! - Linux:  Secret Service / kwallet
//!
//! All values are stored under the service `com.kkape.pier-x` with the
//! caller-provided key as the username.

use thiserror::Error;

use crate::local_secret_store;

const SERVICE: &str = "com.kkape.pier-x";

/// Errors returned by credential operations.
#[derive(Debug, Error)]
pub enum CredentialError {
    /// The OS keyring rejected or could not service the request.
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
    /// The machine-bound local fallback store failed (data dir
    /// unwritable, disk full, …). Only seen when the keyring was already
    /// unavailable, so surfacing it means *both* backends are down.
    #[error("local secret store error: {0}")]
    LocalStore(String),
}

/// Which backend actually holds a value after a [`set_persistent`] call.
/// Returned so callers can audit-log where a secret landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The OS keyring (preferred).
    Keyring,
    /// The machine-bound encrypted local file (keyring was unavailable).
    LocalFile,
}

impl Backend {
    /// Short, log-friendly tag.
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Keyring => "keychain",
            Backend::LocalFile => "local-file",
        }
    }
}

/// Store `value` under `key` in the OS keyring.
pub fn set(key: &str, value: &str) -> Result<(), CredentialError> {
    let entry = keyring::Entry::new(SERVICE, key)?;
    entry.set_password(value)?;
    Ok(())
}

/// Round-trip the OS keyring: write `value`, then immediately read
/// it back. Returns `Ok(true)` only when the read succeeded AND the
/// stored value matches what we just wrote — the caller can use
/// `false` as a signal that the keyring backend silently dropped
/// the write (we have observed Windows Credential Manager behave
/// this way under certain group-policy / sandboxing configurations,
/// and Linux without a running secret-service daemon) and fall
/// back to a non-keychain credential storage path.
pub fn set_and_verify(key: &str, value: &str) -> Result<bool, CredentialError> {
    if let Err(e) = set(key, value) {
        // Treat any explicit Set failure as "keychain unusable" so
        // the caller can take the fallback path instead of
        // surfacing an opaque error to the user. We still log the
        // underlying error so it's diagnosable from the app log.
        log::warn!("keyring set({key}) failed: {e}");
        return Ok(false);
    }
    match get(key) {
        Ok(Some(stored)) => Ok(stored == value),
        Ok(None) => {
            log::warn!("keyring set({key}) succeeded but get() returned None");
            Ok(false)
        }
        Err(e) => {
            log::warn!("keyring verify get({key}) failed: {e}");
            Ok(false)
        }
    }
}

/// Retrieve the value stored under `key`, or `None` if not found.
pub fn get(key: &str) -> Result<Option<String>, CredentialError> {
    let entry = keyring::Entry::new(SERVICE, key)?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Delete the entry stored under `key`. Returns Ok(()) even if the entry
/// did not exist.
pub fn delete(key: &str) -> Result<(), CredentialError> {
    let entry = keyring::Entry::new(SERVICE, key)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Persist `value` under `key`, preferring the OS keyring and falling
/// back to the machine-bound local store ([`local_secret_store`]) when
/// the keyring can't service the write. Returns the [`Backend`] that
/// ended up holding the value so the caller can audit it.
///
/// Why this exists: a plain [`set`] that hits an unavailable keyring
/// either errors or silently drops the write, leaving the secret only in
/// process memory — gone on restart. Routing the *persistent* secrets
/// (today: the elevation password) through here gives them an
/// on-disk home that survives a restart even without a working keyring.
pub fn set_persistent(key: &str, value: &str) -> Result<Backend, CredentialError> {
    // `set_and_verify` returns Ok(false) for the "keyring silently
    // dropped it" case and Err only for a hard keyring error; treat both
    // as "keyring unusable, take the fallback".
    let keyring_ok = matches!(set_and_verify(key, value), Ok(true));
    if keyring_ok {
        // Keyring is now the source of truth — clear any stale local copy
        // so the two backends can't diverge and `get_persistent` can't
        // resurrect an old value after a successful keyring write.
        let _ = local_secret_store::delete(key);
        return Ok(Backend::Keyring);
    }
    local_secret_store::set(key, value).map_err(|e| CredentialError::LocalStore(e.to_string()))?;
    Ok(Backend::LocalFile)
}

/// Read a value written by [`set_persistent`]: keyring first, then the
/// local fallback. The keyring stays authoritative — a value present
/// there wins, and the local copy is only consulted when the keyring has
/// nothing (the "keyring was down when we saved" case).
pub fn get_persistent(key: &str) -> Result<Option<String>, CredentialError> {
    if let Some(value) = get(key)? {
        return Ok(Some(value));
    }
    local_secret_store::get(key).map_err(|e| CredentialError::LocalStore(e.to_string()))
}

/// Remove a value from **both** backends, so a "forget" / clear can't
/// leave a copy behind in whichever store the caller didn't expect.
/// Surfaces a keyring delete error but always attempts the local delete.
pub fn delete_persistent(key: &str) -> Result<(), CredentialError> {
    let keyring_res = delete(key);
    let _ = local_secret_store::delete(key);
    keyring_res
}

/// Build the stable keyring key for a host's privilege-escalation
/// (sudo / `su -`) password. Same `(user, host, port)` triple →
/// same key, so a host accessed under different SSH users keeps
/// distinct elevation entries (matches how sudoers is configured
/// per-user).
///
/// Shape: `pier-x.elev.{user}@{host}:{port}` — the `pier-x.elev.`
/// prefix isolates this namespace from the existing
/// `pier-x.cred-*` SSH credentials and `pier-x.db.*` DB
/// credentials, so a `clearAll`-style wipe of one class can't
/// touch the others.
pub fn elevation_credential_id(user: &str, host: &str, port: u16) -> String {
    format!("pier-x.elev.{user}@{host}:{port}")
}


#[cfg(test)]
mod tests {
    // Note: keyring tests are intentionally not run in CI because they
    // require an unlocked keyring/secret-service which isn't available
    // on the GitHub Actions runners (especially the Linux secret service).
    // Local developers can run them with `cargo test -- --ignored`.

    #[test]
    #[ignore]
    fn round_trip() {
        let key = "pier-x-test";
        super::set(key, "hello").unwrap();
        let got = super::get(key).unwrap();
        assert_eq!(got.as_deref(), Some("hello"));
        super::delete(key).unwrap();
        assert!(super::get(key).unwrap().is_none());
    }
}
