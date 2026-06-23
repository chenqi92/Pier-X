//! Machine-bound encrypted local secret store.
//!
//! This is the **fallback** [`crate::credentials`] reaches for when the
//! OS keyring can't service a write — Linux with no running
//! secret-service / kwallet, or Windows Credential Manager silently
//! dropping the entry under some group-policy / sandbox configs. Without
//! it those hosts fall back to a process-memory cache that evaporates on
//! restart (the "失传" problem); with it, a remembered sudo / elevation
//! password survives across launches.
//!
//! ## Storage
//!
//! Two files under [`crate::paths::data_dir`]:
//! - `.local-secret-key` — a 32-byte AES-256 key, generated once from
//!   the OS CSPRNG and written `0600`. This file IS the machine binding:
//!   it is created locally and never synced, so the encrypted store can
//!   only be decrypted on the machine that minted the key.
//! - `secrets.enc` — JSON `{ version, entries: { key -> hex(nonce ‖ ct) } }`,
//!   each value sealed with AES-256-GCM under a fresh 12-byte nonce.
//!
//! ## Threat model — read this before assuming it's a vault
//!
//! It protects the stored secret against (a) casual inspection (`grep`
//! the file), and (b) copying `secrets.enc` to **another** machine,
//! since the key file stays behind. It does **not** protect against a
//! local attacker who can already run code as this user — they can read
//! the key file too. That's the inherent ceiling of a machine-bound key
//! with no master password; it is the tradeoff deliberately chosen over
//! a master-password scheme. The OS keyring remains the preferred,
//! stronger backend whenever it works — this store is only the graceful
//! degradation path.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const STORE_VERSION: u32 = 1;
/// 32-byte AES key — the machine binding. Dotfile so it stays out of
/// casual directory listings.
const KEY_FILE: &str = ".local-secret-key";
const STORE_FILE: &str = "secrets.enc";
/// AES-GCM standard nonce width.
const NONCE_LEN: usize = 12;

/// On-disk shape of [`STORE_FILE`]. `entries` maps a credential key to
/// `hex(nonce ‖ ciphertext+tag)`.
#[derive(Debug, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    entries: BTreeMap<String, String>,
}

impl Default for StoreFile {
    fn default() -> Self {
        StoreFile {
            version: STORE_VERSION,
            entries: BTreeMap::new(),
        }
    }
}

fn io_other<E: ToString>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}

fn data_dir() -> io::Result<PathBuf> {
    crate::paths::data_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no application data directory"))
}

/// Write `bytes` to `path` with owner-only permissions where the OS
/// supports it (Unix `0600`). On Windows the per-user data directory
/// already carries a profile-scoped ACL, so a plain write is owner-only
/// in practice.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(bytes)
    }
    #[cfg(not(unix))]
    {
        fs::write(path, bytes)
    }
}

/// Load the machine key, minting it on first use. Returned in a
/// [`Zeroizing`] wrapper so it's wiped from memory on drop.
fn load_or_create_key() -> io::Result<Zeroizing<[u8; 32]>> {
    let dir = data_dir()?;
    let path = dir.join(KEY_FILE);
    if let Ok(bytes) = fs::read(&path) {
        if bytes.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            return Ok(Zeroizing::new(k));
        }
        // Wrong size → treat as corrupt and regenerate. Any entries
        // sealed under the old key become undecryptable (callers degrade
        // to "no stored secret" and re-prompt), which is the safe
        // failure here.
    }
    fs::create_dir_all(&dir)?;
    let mut key = Zeroizing::new([0u8; 32]);
    getrandom::getrandom(&mut key[..]).map_err(io_other)?;
    write_private(&path, &key[..])?;
    Ok(key)
}

fn cipher() -> io::Result<Aes256Gcm> {
    let key = load_or_create_key()?;
    let key = Key::<Aes256Gcm>::from_slice(&key[..]);
    Ok(Aes256Gcm::new(key))
}

fn store_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.join(STORE_FILE))
}

/// Read + parse the store. `Ok(None)` = the file is absent; `Ok(Some(_))`
/// = parsed cleanly; `Err` = present but unreadable/unparseable. Writers
/// must distinguish these so a corrupt file is never treated as "empty"
/// and blindly overwritten — which would drop every other stored secret.
fn read_store_file() -> io::Result<Option<StoreFile>> {
    let path = store_path()?;
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("secret store is corrupt: {e}"),
        )
    })
}

/// Lenient load for read-only callers ([`get`]): a missing or corrupt file
/// degrades to an empty store (caller treats it as "no stored secret" and
/// re-prompts). Never writes, so it cannot clobber anything.
fn load_store_lenient() -> StoreFile {
    read_store_file().ok().flatten().unwrap_or_default()
}

/// Load for a read-modify-write ([`set`] / [`delete`]). On a corrupt file
/// it quarantines the bad file to `secrets.enc.corrupt` and logs, so the
/// next write starts clean instead of silently overwriting (and thereby
/// destroying) the other entries. If the bad file can't even be moved
/// aside, it refuses rather than risk clobbering it.
fn load_store_for_write() -> io::Result<StoreFile> {
    match read_store_file() {
        Ok(Some(store)) => Ok(store),
        Ok(None) => Ok(StoreFile::default()),
        Err(e) => {
            let path = store_path()?;
            let aside = path.with_file_name(format!("{STORE_FILE}.corrupt"));
            match fs::rename(&path, &aside) {
                Ok(()) => {
                    log::warn!(
                        "local secret store was corrupt ({e}); quarantined to {}",
                        aside.display()
                    );
                    Ok(StoreFile::default())
                }
                Err(re) => Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "secret store is corrupt and could not be quarantined: {re} (original: {e})"
                    ),
                )),
            }
        }
    }
}

/// Persist `store` atomically: write to a sibling temp file, fsync, then
/// rename over the target. A crash leaves either the old or the new file
/// fully intact — never a truncated mix that the next load would reject.
fn save_store(store: &StoreFile) -> io::Result<()> {
    let dir = data_dir()?;
    fs::create_dir_all(&dir)?;
    let bytes = serde_json::to_vec_pretty(store).map_err(io_other)?;
    let final_path = dir.join(STORE_FILE);
    let tmp_path = dir.join(format!("{STORE_FILE}.tmp"));
    write_private(&tmp_path, &bytes)?;
    // Make the temp file's bytes durable before the rename swaps it in.
    if let Ok(f) = fs::File::open(&tmp_path) {
        let _ = f.sync_all();
    }
    fs::rename(&tmp_path, &final_path)
}

/// Seal `value` under `key`. Overwrites any existing entry.
pub fn set(key: &str, value: &str) -> io::Result<()> {
    let cipher = cipher()?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce_bytes).map_err(io_other)?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, value.as_bytes())
        .map_err(|_| io_other("AES-GCM encryption failed"))?;

    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);

    let mut store = load_store_for_write()?;
    store.version = STORE_VERSION;
    store.entries.insert(key.to_string(), hex::encode(&blob));
    save_store(&store)
}

/// Open the sealed value for `key`. Returns `None` when there's no
/// entry, or when the entry can't be decrypted (key rotated, file
/// tampered) — callers treat both as "no stored secret" and re-prompt.
pub fn get(key: &str) -> io::Result<Option<String>> {
    let store = load_store_lenient();
    let Some(hexed) = store.entries.get(key) else {
        return Ok(None);
    };
    let Ok(blob) = hex::decode(hexed) else {
        return Ok(None);
    };
    if blob.len() <= NONCE_LEN {
        return Ok(None);
    }
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = cipher()?;
    let nonce = Nonce::from_slice(nonce_bytes);
    match cipher.decrypt(nonce, ciphertext) {
        Ok(plain) => Ok(Some(String::from_utf8_lossy(&plain).into_owned())),
        Err(_) => Ok(None),
    }
}

/// Drop the entry for `key`. A missing entry is not an error.
pub fn delete(key: &str) -> io::Result<()> {
    let mut store = load_store_for_write()?;
    if store.entries.remove(key).is_some() {
        save_store(&store)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // These touch the real data dir, so they're opt-in (`--ignored`) to
    // keep CI hermetic — same stance as the keyring round-trip test.
    #[test]
    #[ignore]
    fn round_trip_set_get_delete() {
        let key = "pier-x.test.local-store";
        set(key, "hunter2").unwrap();
        assert_eq!(get(key).unwrap().as_deref(), Some("hunter2"));
        // Overwrite with a fresh nonce.
        set(key, "second").unwrap();
        assert_eq!(get(key).unwrap().as_deref(), Some("second"));
        delete(key).unwrap();
        assert_eq!(get(key).unwrap(), None);
    }

    #[test]
    #[ignore]
    fn missing_key_is_none() {
        assert_eq!(get("pier-x.test.does-not-exist").unwrap(), None);
    }
}
