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

const SERVICE: &str = "com.kkape.pier-x";

/// Errors returned by credential operations.
#[derive(Debug, Error)]
pub enum CredentialError {
    /// The OS keyring rejected or could not service the request.
    #[error("keyring error: {0}")]
    Keyring(#[from] keyring::Error),
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
