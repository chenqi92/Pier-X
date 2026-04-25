//! Process-level SSH credential cache.
//!
//! What this gives the user: when they answer a password prompt or
//! a key passphrase prompt in a Pier-X terminal tab, that secret is
//! mirrored into a process-wide cache keyed by `(host, port, user)`.
//! Every right-side panel (firewall, monitor, SFTP, Docker, DB
//! tunnels) goes through `russh` — a different SSH client from the
//! one running in the PTY — so without this cache they had no way
//! to reuse the credential the user just typed and would either
//! prompt the user again or fail with "auth rejected".
//!
//! Why a process-level cache, not per-tab:
//!   * Multiple tabs on the same target should share — typing the
//!     password in tab #1 should let tab #2's right panel connect.
//!   * Same target reached via "Saved Connection" sidebar vs. an
//!     ad-hoc terminal `ssh user@host` should share too.
//!   * A new tab opened to a target the user reached 5 minutes ago
//!     should still get the cached cred (within TTL).
//!
//! Why not persist to disk: secrets in plaintext on disk is the
//! single biggest mistake an SSH GUI can make. The OS keychain
//! path already exists for "I want this remembered across app
//! restarts" (saved connections); this cache is for "I just typed
//! it, don't make me type it twice in the same session". Process
//! exit clears everything.
//!
//! TTL: 30 minutes default, sliding window — every read of an entry
//! refreshes its `last_used`, so an actively-used target keeps its
//! credential indefinitely while a one-off connection drops out
//! after 30 minutes idle. Forced eviction is via [`forget`].

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default TTL — long enough that a user can step away from the
/// keyboard for half an hour without re-typing, short enough that
/// a forgotten laptop doesn't keep the secret indefinitely.
pub const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);

/// The cache key. Port is normalised at insert time (0 → 22) so
/// `(host, 0, user)` and `(host, 22, user)` collapse — they're the
/// same target from any user-facing perspective.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct TargetKey {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl TargetKey {
    pub fn new(host: &str, port: u16, user: &str) -> Self {
        Self {
            host: host.trim().to_string(),
            port: if port == 0 { 22 } else { port },
            user: user.trim().to_string(),
        }
    }
}

/// One entry: whatever credentials we've harvested for this target,
/// plus when it was last touched (for sliding-window TTL).
#[derive(Clone, Debug)]
pub struct CachedCred {
    /// Server password — what the user typed at OpenSSH's
    /// `<user>@<host>'s password:` prompt, or what saved-connection
    /// resolved from keychain.
    pub password: Option<String>,
    /// Passphrase for an encrypted private key — what the user
    /// typed at OpenSSH's `Enter passphrase for key '<path>':`
    /// prompt. Lives separately from `password` because passing a
    /// key passphrase as a server password (or vice versa) is the
    /// kind of bug that wastes everyone's afternoon.
    pub key_passphrase: Option<String>,
    /// Optional explicit private key path (`-i <path>`), so right-
    /// side russh sessions can attempt the same key the terminal's
    /// ssh did.
    pub key_path: Option<String>,
    last_used: Instant,
}

impl CachedCred {
    fn fresh() -> Self {
        Self {
            password: None,
            key_passphrase: None,
            key_path: None,
            last_used: Instant::now(),
        }
    }
}

/// Thread-safe credential map. Cheap to clone (Arc-backed via the
/// Mutex's inner data structure pointer). One instance lives in
/// AppState; all callers share it.
pub struct SshCredCache {
    inner: Mutex<HashMap<TargetKey, CachedCred>>,
    ttl: Duration,
}

impl Default for SshCredCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl: DEFAULT_TTL,
        }
    }
}

impl SshCredCache {
    /// Look up by target. Returns a snapshot clone and refreshes
    /// `last_used` on hit (sliding window).
    pub fn get(&self, key: &TargetKey) -> Option<CachedCred> {
        let mut guard = self.inner.lock().ok()?;
        let entry = guard.get_mut(key)?;
        if entry.last_used.elapsed() > self.ttl {
            // TTL expired — evict and report miss.
            guard.remove(key);
            return None;
        }
        entry.last_used = Instant::now();
        Some(entry.clone())
    }

    /// Merge a captured password into the entry for `key`, creating
    /// the entry if absent. Empty passwords are silently ignored
    /// (nothing useful to remember). Returns whether anything
    /// actually changed.
    pub fn put_password(&self, key: TargetKey, password: &str) -> bool {
        if password.is_empty() {
            return false;
        }
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let entry = guard.entry(key).or_insert_with(CachedCred::fresh);
        let changed = entry.password.as_deref() != Some(password);
        entry.password = Some(password.to_string());
        entry.last_used = Instant::now();
        changed
    }

    /// Merge a captured key passphrase into the entry. Same shape
    /// as [`Self::put_password`] but writes the `key_passphrase`
    /// slot.
    pub fn put_passphrase(&self, key: TargetKey, passphrase: &str) -> bool {
        if passphrase.is_empty() {
            return false;
        }
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let entry = guard.entry(key).or_insert_with(CachedCred::fresh);
        let changed = entry.key_passphrase.as_deref() != Some(passphrase);
        entry.key_passphrase = Some(passphrase.to_string());
        entry.last_used = Instant::now();
        changed
    }

    /// Remember the explicit key path (-i <path>) the terminal used.
    pub fn put_key_path(&self, key: TargetKey, key_path: &str) -> bool {
        if key_path.is_empty() {
            return false;
        }
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let entry = guard.entry(key).or_insert_with(CachedCred::fresh);
        let changed = entry.key_path.as_deref() != Some(key_path);
        entry.key_path = Some(key_path.to_string());
        entry.last_used = Instant::now();
        changed
    }

    /// Drop everything we know about `key`. Used by:
    ///   * The "Forget this target" UI affordance.
    ///   * The watcher when an ssh child exits and the remote/user
    ///     pair changes (so a new `ssh otheruser@samehost` doesn't
    ///     pick up the previous user's password).
    pub fn forget(&self, key: &TargetKey) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.remove(key);
        }
    }

    /// Clear every cached credential. Wired to `ssh_mux_shutdown_all`
    /// in the app exit path so we never leak secrets past the
    /// process boundary.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.clear();
        }
    }

    /// Sweep entries past TTL. Called opportunistically on the slow
    /// path of `get_or_open_ssh_session` so the cache doesn't grow
    /// unbounded for users who roam between many hosts.
    pub fn prune_expired(&self) {
        if let Ok(mut guard) = self.inner.lock() {
            let ttl = self.ttl;
            guard.retain(|_, entry| entry.last_used.elapsed() <= ttl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_and_get_round_trip() {
        let cache = SshCredCache::default();
        let key = TargetKey::new("example.com", 22, "alice");
        assert!(cache.get(&key).is_none());
        cache.put_password(key.clone(), "hunter2");
        let got = cache.get(&key).expect("hit");
        assert_eq!(got.password.as_deref(), Some("hunter2"));
        assert!(got.key_passphrase.is_none());
    }

    #[test]
    fn empty_secrets_are_ignored() {
        let cache = SshCredCache::default();
        let key = TargetKey::new("example.com", 22, "alice");
        assert!(!cache.put_password(key.clone(), ""));
        assert!(!cache.put_passphrase(key.clone(), ""));
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn port_zero_normalises_to_22() {
        let cache = SshCredCache::default();
        cache.put_password(TargetKey::new("h", 0, "u"), "p");
        assert!(cache.get(&TargetKey::new("h", 22, "u")).is_some());
    }

    #[test]
    fn forget_removes_entry() {
        let cache = SshCredCache::default();
        let key = TargetKey::new("h", 22, "u");
        cache.put_password(key.clone(), "p");
        cache.forget(&key);
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn separate_password_and_passphrase_slots() {
        let cache = SshCredCache::default();
        let key = TargetKey::new("h", 22, "u");
        cache.put_password(key.clone(), "server-pw");
        cache.put_passphrase(key.clone(), "key-pp");
        let got = cache.get(&key).expect("hit");
        assert_eq!(got.password.as_deref(), Some("server-pw"));
        assert_eq!(got.key_passphrase.as_deref(), Some("key-pp"));
    }
}
