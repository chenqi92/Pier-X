//! TOFU certificate pinning for RDP — the remote-desktop analog of SSH
//! `known_hosts`.
//!
//! The RDP backends would otherwise accept *any* server certificate, so an
//! on-path attacker could impersonate a host and observe/inject the session.
//! This module pins the server's public key on first contact and compares it
//! on every reconnect, exactly like SSH host keys: first contact learns the
//! key (or prompts when a callback is wired), a later mismatch is refused
//! (or prompts). Pins live in a small JSON file under the app data dir.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STORE_FILE: &str = "rdp_known_certs.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct PinFile {
    /// `host:port` → `SHA256:<hex>` of the pinned server public key.
    pins: BTreeMap<String, String>,
}

/// Outcome of comparing a presented key against the stored pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinCheck {
    /// Matches the stored pin — trusted.
    Match,
    /// No pin recorded yet — first contact (TOFU).
    Unknown,
    /// A pin exists but differs from what was presented — possible MITM.
    Mismatch,
}

/// `SHA256:<hex>` fingerprint of `bytes` (the server SubjectPublicKeyInfo).
pub fn fingerprint(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(7 + digest.len() * 2);
    s.push_str("SHA256:");
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn entry_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

fn store_path() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join(STORE_FILE))
}

fn load() -> PinFile {
    let Some(path) = store_path() else {
        return PinFile::default();
    };
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => PinFile::default(),
    }
}

fn persist(file: &PinFile) -> io::Result<()> {
    let Some(path) = store_path() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let bytes = serde_json::to_vec_pretty(file).map_err(|e| io::Error::other(e.to_string()))?;
    // Write to a temp sibling then rename so a crash can't truncate the pins.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes)?;
    fs::rename(&tmp, &path)
}

/// Compare `fingerprint` against the stored pin for `host:port`.
pub fn check(host: &str, port: u16, fingerprint: &str) -> PinCheck {
    match load().pins.get(&entry_key(host, port)) {
        None => PinCheck::Unknown,
        Some(stored) if stored == fingerprint => PinCheck::Match,
        Some(_) => PinCheck::Mismatch,
    }
}

/// Record (or replace) the pin for `host:port`. Best-effort: a write failure
/// is logged, not fatal — the connection still proceeds for this session.
pub fn save(host: &str, port: u16, fingerprint: &str) {
    let mut file = load();
    file.pins
        .insert(entry_key(host, port), fingerprint.to_string());
    if let Err(e) = persist(&file) {
        log::warn!("failed to persist RDP cert pin for {host}:{port}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_prefixed() {
        let a = fingerprint(b"hello");
        assert!(a.starts_with("SHA256:"));
        assert_eq!(a, fingerprint(b"hello"));
        assert_ne!(a, fingerprint(b"world"));
        // 7-char prefix + 64 hex chars.
        assert_eq!(a.len(), 7 + 64);
    }
}
