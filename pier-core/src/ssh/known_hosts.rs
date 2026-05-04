//! Host key verification.
//!
//! ## M3b — interactive TOFU + mismatch prompts
//!
//! Earlier milestones shipped `OpenSshKnownHosts` with silent
//! `accept-new` (auto-learn) semantics: any host not pinned in
//! the file was learned without asking, mismatches always blocked.
//! M3b adds an optional `prompt` callback: when set, unknown
//! hosts and changed-key hosts both go through the user before
//! the connection completes. The callback returns
//! [`HostKeyDecision::Accept`] (learn / replace + accept) or
//! [`HostKeyDecision::Reject`] (block).
//!
//! When `prompt` is unset (the default for tests + sync code
//! paths), behaviour is exactly the pre-M3b silent TOFU — kept
//! for backward compatibility and to avoid making every test
//! drag in an async runtime.
//!
//! ## What does NOT live here
//!
//! The dialog itself. The Tauri layer constructs a callback that
//! emits an event to the React frontend, awaits a oneshot
//! response, and returns the decision — `pier-core` stays
//! UI-agnostic.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use russh::keys::ssh_key::PublicKey;

use crate::paths;

/// Decision the prompt callback hands back. `Accept` causes a
/// learn (for unknown hosts) or a replace-and-learn (for
/// changed hosts); `Reject` causes the connect to fail with
/// the appropriate typed error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyDecision {
    /// Trust this host key. For unknown hosts the verifier
    /// learns the key (TOFU); for changed hosts it removes the
    /// stale pin and writes the new one.
    Accept,
    /// Refuse this host key. The connect fails with
    /// [`super::SshError::HostKeyRejected`] (unknown) or
    /// [`super::SshError::HostKeyMismatch`] (changed).
    Reject,
}

/// What the verifier wants the user to decide. Carried into
/// the prompt callback so the UI can render the right copy
/// ("trust this new host?" vs "host key changed!").
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPromptRequest {
    /// Hostname being dialed.
    pub host: String,
    /// Port being dialed.
    pub port: u16,
    /// SSH key algorithm name (e.g. `ssh-ed25519`).
    pub key_type: String,
    /// SHA-256 fingerprint of the server-presented key
    /// (`SHA256:abcd…`).
    pub fingerprint: String,
    /// Why we're asking.
    pub kind: HostKeyPromptKind,
}

/// Reason for the prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HostKeyPromptKind {
    /// No existing pin for this host — TOFU first contact.
    Unknown,
    /// The pinned key for this host doesn't match the
    /// presented one.
    Changed,
}

/// Async callback the verifier consults for unknown / changed
/// hosts. The Tauri layer wires this to a "trust this host?"
/// dialog; tests pass `None` and fall back to silent TOFU.
pub type HostKeyPromptCb = Arc<
    dyn Fn(HostKeyPromptRequest) -> Pin<Box<dyn Future<Output = HostKeyDecision> + Send + 'static>>
        + Send
        + Sync,
>;

/// Where the verifier reads pins from.
#[derive(Debug, Clone)]
pub enum HostKeySource {
    /// Parse an OpenSSH-format known_hosts file. `path` is
    /// created on first write if it does not yet exist.
    OpenSshKnownHosts {
        /// Absolute path to the known_hosts file.
        path: PathBuf,
    },

    /// Accept every server key and log the fingerprint. Tests
    /// use this to avoid touching the user's real known_hosts
    /// file; production builds should not.
    AcceptAllLogFingerprint,
}

/// How an SSH session decides whether to trust a server's host
/// key. Cheap to clone — the optional prompt callback is an
/// `Arc`.
#[derive(Clone)]
pub struct HostKeyVerifier {
    /// Where the pinned keys live (or `AcceptAll` for the
    /// test/escape-hatch variant).
    pub source: HostKeySource,
    /// Optional async callback. Consulted by `verify_async`
    /// for unknown / changed hosts; left unset, the verifier
    /// silently learns first-contact keys and rejects
    /// mismatches.
    pub prompt: Option<HostKeyPromptCb>,
}

impl std::fmt::Debug for HostKeyVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostKeyVerifier")
            .field("source", &self.source)
            .field("prompt", &self.prompt.as_ref().map(|_| "<fn>"))
            .finish()
    }
}

impl HostKeyVerifier {
    /// Construct an `OpenSshKnownHosts` verifier with no prompt
    /// — silent TOFU on unknown hosts, hard error on mismatch.
    pub fn open_ssh_known_hosts(path: PathBuf) -> Self {
        Self {
            source: HostKeySource::OpenSshKnownHosts { path },
            prompt: None,
        }
    }

    /// Construct the test/escape-hatch verifier that accepts
    /// any key.
    pub fn accept_all_log_fingerprint() -> Self {
        Self {
            source: HostKeySource::AcceptAllLogFingerprint,
            prompt: None,
        }
    }

    /// Attach a prompt callback. When set, unknown hosts and
    /// changed-key hosts both round-trip to the user before
    /// the connection proceeds.
    pub fn with_prompt(mut self, prompt: HostKeyPromptCb) -> Self {
        self.prompt = Some(prompt);
        self
    }

    /// Sync verifier — silent TOFU on unknown, hard error on
    /// mismatch. The prompt callback is not consulted (sync code
    /// has no runtime to await on). Used by tests and by any
    /// caller that explicitly wants the legacy `accept-new`
    /// behaviour.
    pub fn verify(&self, host: &str, port: u16, key: &PublicKey) -> Result<bool, VerifyError> {
        match self.preflight(host, port, key)? {
            Decision::Accept => Ok(true),
            Decision::NeedsLearn => {
                self.learn(host, port, key)?;
                Ok(true)
            }
            Decision::NeedsReplace { line, fingerprint } => Err(VerifyError::Mismatch {
                host: host.to_string(),
                fingerprint,
                line,
            }),
        }
    }

    /// Async verifier — consults the prompt callback (if set)
    /// before learning a new host or replacing a changed pin.
    /// Used by [`super::session::SshSession`] from inside the
    /// russh client handler.
    pub async fn verify_async(
        &self,
        host: &str,
        port: u16,
        key: &PublicKey,
    ) -> Result<bool, VerifyError> {
        let outcome = self.preflight(host, port, key)?;
        match outcome {
            Decision::Accept => Ok(true),
            Decision::NeedsLearn => match &self.prompt {
                Some(prompt) => {
                    let req = HostKeyPromptRequest {
                        host: host.to_string(),
                        port,
                        key_type: key.algorithm().to_string(),
                        fingerprint: format!("{}", key.fingerprint(russh::keys::HashAlg::Sha256)),
                        kind: HostKeyPromptKind::Unknown,
                    };
                    match prompt(req).await {
                        HostKeyDecision::Accept => {
                            self.learn(host, port, key)?;
                            Ok(true)
                        }
                        HostKeyDecision::Reject => Err(VerifyError::UserRejected {
                            host: host.to_string(),
                            kind: HostKeyPromptKind::Unknown,
                        }),
                    }
                }
                None => {
                    // Legacy: silent TOFU.
                    self.learn(host, port, key)?;
                    Ok(true)
                }
            },
            Decision::NeedsReplace { line, fingerprint } => match &self.prompt {
                Some(prompt) => {
                    let req = HostKeyPromptRequest {
                        host: host.to_string(),
                        port,
                        key_type: key.algorithm().to_string(),
                        fingerprint: fingerprint.clone(),
                        kind: HostKeyPromptKind::Changed,
                    };
                    match prompt(req).await {
                        HostKeyDecision::Accept => {
                            // Drop the conflicting pin first, then
                            // append the new one. Subsequent
                            // verifications match the new key by
                            // value; the line-number shift from
                            // removing the old line is harmless.
                            self.replace(host, port, key, line)?;
                            Ok(true)
                        }
                        HostKeyDecision::Reject => Err(VerifyError::Mismatch {
                            host: host.to_string(),
                            fingerprint,
                            line,
                        }),
                    }
                }
                None => Err(VerifyError::Mismatch {
                    host: host.to_string(),
                    fingerprint,
                    line,
                }),
            },
        }
    }

    /// Look up `(host, port, key)` in the configured source
    /// without mutating anything — returns one of the three
    /// outcomes the sync / async paths then act on.
    fn preflight(
        &self,
        host: &str,
        port: u16,
        key: &PublicKey,
    ) -> Result<Decision, VerifyError> {
        match &self.source {
            HostKeySource::AcceptAllLogFingerprint => {
                let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256);
                log::info!(
                    "ssh host key for {host}:{port} (AcceptAll, escape-hatch): {fingerprint}",
                );
                Ok(Decision::Accept)
            }
            HostKeySource::OpenSshKnownHosts { path } => {
                match russh::keys::known_hosts::check_known_hosts_path(host, port, key, path) {
                    Ok(true) => {
                        log::debug!("host key for {host}:{port} matches known_hosts pin");
                        Ok(Decision::Accept)
                    }
                    Ok(false) => Ok(Decision::NeedsLearn),
                    Err(russh::keys::Error::KeyChanged { line }) => {
                        let fingerprint =
                            format!("{}", key.fingerprint(russh::keys::HashAlg::Sha256));
                        log::warn!(
                            "host key for {host}:{port} MISMATCH at {path:?} line {line}: {fingerprint}",
                        );
                        Ok(Decision::NeedsReplace { line, fingerprint })
                    }
                    Err(e) => Err(VerifyError::Io(format!("{e}"))),
                }
            }
        }
    }

    /// Append `key` to the configured source. Best-effort
    /// creates `~/.ssh` (or the equivalent parent) with mode
    /// 0700 first to match `ssh-keygen` behaviour on a fresh
    /// machine.
    fn learn(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), VerifyError> {
        let path = match &self.source {
            HostKeySource::OpenSshKnownHosts { path } => path,
            HostKeySource::AcceptAllLogFingerprint => return Ok(()),
        };
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    log::warn!("failed to create known_hosts parent {parent:?}: {e}",);
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        parent,
                        std::fs::Permissions::from_mode(0o700),
                    );
                }
            }
        }
        russh::keys::known_hosts::learn_known_hosts_path(host, port, key, path)
            .map_err(|e| VerifyError::Io(format!("{e}")))?;
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256);
        log::info!("learned new host key for {host}:{port}: {fingerprint}",);
        Ok(())
    }

    /// Drop the entry on `line` and append `key` as a fresh
    /// pin. Used after the user accepts a changed-host prompt.
    fn replace(
        &self,
        host: &str,
        port: u16,
        key: &PublicKey,
        line: usize,
    ) -> Result<(), VerifyError> {
        let path = match &self.source {
            HostKeySource::OpenSshKnownHosts { path } => path.clone(),
            HostKeySource::AcceptAllLogFingerprint => return Ok(()),
        };
        remove_known_host_line(&path, line).map_err(|e| VerifyError::Io(format!("{e}")))?;
        self.learn(host, port, key)
    }
}

/// Internal: outcome of a non-mutating known_hosts lookup.
enum Decision {
    Accept,
    NeedsLearn,
    NeedsReplace { line: usize, fingerprint: String },
}

impl Default for HostKeyVerifier {
    fn default() -> Self {
        // Default to the real verifier pointing at the
        // OpenSSH-standard location. If the platform doesn't
        // expose a home directory (which should never happen in
        // practice on Unix / Windows / macOS), fall back to the
        // accept-all variant and log — failing loudly here would
        // keep pier-x from running at all on an unusual setup.
        match default_known_hosts_path() {
            Some(path) => Self::open_ssh_known_hosts(path),
            None => {
                log::warn!("no resolvable home directory; falling back to AcceptAllLogFingerprint",);
                Self::accept_all_log_fingerprint()
            }
        }
    }
}

/// Resolve `~/.ssh/known_hosts` for the current user without
/// pulling in any OpenSSH-specific dependencies.
pub fn default_known_hosts_path() -> Option<PathBuf> {
    // directories::UserDirs would be cleaner but we already
    // depend on directories for data_dir(); the existing
    // paths module doesn't expose a home-dir helper though,
    // so do the simple env-based lookup here. Works on both
    // Unix ($HOME) and Windows (%USERPROFILE% via the same env
    // var on most installations), and has no fallible parse
    // step that would surface a confusing error.
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".ssh").join("known_hosts"))
}

/// Per-file override: write the known_hosts path into the
/// pier-x data directory instead of `~/.ssh/known_hosts`.
/// Used by the (not-yet-built) "isolated profile" mode that
/// keeps pier-x from touching the user's real SSH state.
#[allow(dead_code)]
pub fn pier_x_known_hosts_path() -> Option<PathBuf> {
    paths::data_dir().map(|d| d.join("known_hosts"))
}

/// A single parsed `known_hosts` entry. Produced by
/// [`list_known_hosts`] for display in the Settings UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostEntry {
    /// 1-based line number in the file. Used as a stable
    /// identifier for [`remove_known_host_line`].
    pub line: usize,
    /// Raw host prefix from the file. For hashed entries this is
    /// opaque (`|1|salt|hash`); for plain entries it's the
    /// comma-separated hostname list.
    pub host: String,
    /// SSH key type label (`ssh-ed25519`, `ecdsa-sha2-nistp256`,
    /// `ssh-rsa`, ...). Empty if the line is malformed.
    pub key_type: String,
    /// Human-readable SHA-256 fingerprint (`SHA256:...`). Empty
    /// when the key can't be decoded.
    pub fingerprint: String,
    /// True when the host prefix is OpenSSH-hashed. Hashed
    /// entries display as "hashed" — the original hostname
    /// isn't recoverable.
    pub hashed: bool,
}

/// Parse an OpenSSH-format known_hosts file and return one entry
/// per non-comment, non-empty line. Malformed lines are skipped
/// silently — the canonical OpenSSH tools do the same.
///
/// Returns `Ok(Vec::new())` when the file does not exist; callers
/// generally want to treat "no file" as "no pins" rather than an
/// error.
pub fn list_known_hosts(path: &std::path::Path) -> std::io::Result<Vec<KnownHostEntry>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut out = Vec::new();
    for (idx, raw) in contents.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // Format: <host>[,host...] <keytype> <base64> [comment]
        // Fields are whitespace-separated; the comment tail may
        // contain arbitrary text so we only split on the first
        // three boundaries.
        let mut parts = trimmed.splitn(3, char::is_whitespace);
        let host = match parts.next() {
            Some(h) => h.to_string(),
            None => continue,
        };
        let key_type = parts.next().unwrap_or("").to_string();
        let remainder = parts.next().unwrap_or("");
        // The rest is `<base64> [comment]` — strip the comment
        // before feeding to russh's parser so it gets a clean
        // `<keytype> <base64>` line.
        let base64 = remainder.split_whitespace().next().unwrap_or("");
        let openssh_line = format!("{key_type} {base64}");
        let fingerprint = russh::keys::ssh_key::PublicKey::from_openssh(&openssh_line)
            .map(|k| format!("{}", k.fingerprint(russh::keys::HashAlg::Sha256)))
            .unwrap_or_default();
        out.push(KnownHostEntry {
            line: line_no,
            hashed: host.starts_with("|1|"),
            host,
            key_type,
            fingerprint,
        });
    }
    Ok(out)
}

/// Remove the entry on `line_no` (1-based) from a known_hosts
/// file. Preserves all other lines (including comments and
/// blank lines) verbatim. Silently succeeds if the file does
/// not exist or the line is out of range — matches the
/// "best-effort cleanup" semantics the UI expects.
pub fn remove_known_host_line(path: &std::path::Path, line_no: usize) -> std::io::Result<()> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if line_no == 0 {
        return Ok(());
    }
    let mut kept = Vec::with_capacity(contents.lines().count());
    for (idx, raw) in contents.lines().enumerate() {
        if idx + 1 == line_no {
            continue;
        }
        kept.push(raw);
    }
    let trailing_newline = contents.ends_with('\n');
    let mut rewritten = kept.join("\n");
    if trailing_newline && !rewritten.is_empty() {
        rewritten.push('\n');
    }
    std::fs::write(path, rewritten)
}

/// Errors the verifier can produce. Separate from
/// [`crate::ssh::SshError`] so the `SshSession::connect`
/// handler can match on `Mismatch` / `UserRejected`
/// specifically and translate to the matching
/// [`crate::ssh::SshError`] variant.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    /// An existing known_hosts entry for this host does not
    /// match the server-presented key. Surface to the user
    /// as a security warning.
    #[error("host key mismatch for {host} at line {line}: {fingerprint}")]
    Mismatch {
        /// Hostname that was being dialed.
        host: String,
        /// SHA-256 fingerprint of the new key the server presented.
        fingerprint: String,
        /// Line number in known_hosts of the conflicting entry.
        line: usize,
    },

    /// The user explicitly declined a TOFU / changed-host
    /// prompt — the connect must abort. Distinct from
    /// `Mismatch` so the UI can phrase the failure as "you
    /// rejected" rather than re-showing the security warning.
    #[error("host key rejected by user for {host}")]
    UserRejected {
        /// Hostname the user declined to trust.
        host: String,
        /// Whether the prompt was for a first-contact (`Unknown`)
        /// host or a key-changed host.
        kind: HostKeyPromptKind,
    },

    /// I/O or parse error reading the known_hosts file.
    #[error("known_hosts verifier I/O: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn fresh_tmp_path(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        temp_dir().join(format!("pier-x-test-khosts-{label}-{pid}-{n}"))
    }

    /// Build a deterministic ed25519 public key for tests from
    /// a single seed byte. The returned `PublicKey` has an
    /// **empty comment** — russh's `learn_known_hosts_path`
    /// writes the key with its comment and `check_known_hosts_path`
    /// reads it back via `parse_public_key_base64` which
    /// discards the comment, so test keys with non-empty
    /// comments would round-trip as "not equal" to themselves
    /// and break the TOFU test path. Empty comments round-trip
    /// identically.
    fn make_test_key(seed: u8) -> PublicKey {
        use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
        use russh::keys::ssh_key::PrivateKey;

        let seed_bytes = [seed; 32];
        let keypair = Ed25519Keypair::from_seed(&seed_bytes);
        let pk = PrivateKey::new(KeypairData::Ed25519(keypair), "")
            .expect("constructing ed25519 PrivateKey from seed must succeed");
        pk.public_key().clone()
    }

    #[test]
    fn accept_all_variant_always_returns_true() {
        let v = HostKeyVerifier::accept_all_log_fingerprint();
        let key = make_test_key(1);
        assert!(v.verify("example.com", 22, &key).unwrap());
    }

    #[test]
    fn opensshkh_tofu_learns_and_then_accepts() {
        let path = fresh_tmp_path("tofu");
        let _ = fs::remove_file(&path);

        let verifier = HostKeyVerifier::open_ssh_known_hosts(path.clone());
        let key = make_test_key(2);

        // First verify: file doesn't exist → learned + accepted.
        assert!(verifier.verify("first.example.com", 22, &key).unwrap());
        assert!(path.exists(), "TOFU learn must create the file");

        // Second verify for the same host+key: matches → accepted
        // without touching the file.
        assert!(verifier.verify("first.example.com", 22, &key).unwrap());

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn opensshkh_mismatch_is_reported_structurally() {
        let path = fresh_tmp_path("mismatch");
        let _ = fs::remove_file(&path);

        let verifier = HostKeyVerifier::open_ssh_known_hosts(path.clone());
        let key_a = make_test_key(3);
        let key_b = make_test_key(4);

        assert!(verifier.verify("mismatch.example.com", 22, &key_a).unwrap());
        let err = verifier
            .verify("mismatch.example.com", 22, &key_b)
            .expect_err("mismatch must surface as Err(VerifyError::Mismatch)");
        match err {
            VerifyError::Mismatch {
                host, fingerprint, ..
            } => {
                assert_eq!(host, "mismatch.example.com");
                assert!(
                    fingerprint.contains("SHA256:"),
                    "expected SHA256-prefixed fingerprint, got {fingerprint:?}",
                );
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn default_resolves_to_openssh_path() {
        let v = HostKeyVerifier::default();
        assert!(
            matches!(v.source, HostKeySource::OpenSshKnownHosts { .. }),
            "default must be the real verifier, not {v:?}",
        );
        assert!(v.prompt.is_none(), "default must not carry a prompt");
    }

    #[tokio::test]
    async fn verify_async_consults_prompt_for_unknown_host() {
        let path = fresh_tmp_path("async-unknown");
        let _ = fs::remove_file(&path);
        let key = make_test_key(5);

        let calls = Arc::new(std::sync::Mutex::new(0u32));
        let calls_cb = calls.clone();
        let prompt: HostKeyPromptCb = Arc::new(move |req| {
            assert_eq!(req.kind, HostKeyPromptKind::Unknown);
            *calls_cb.lock().unwrap() += 1;
            Box::pin(async move { HostKeyDecision::Accept })
        });

        let verifier = HostKeyVerifier::open_ssh_known_hosts(path.clone()).with_prompt(prompt);
        assert!(verifier
            .verify_async("u.example.com", 22, &key)
            .await
            .unwrap());
        assert_eq!(*calls.lock().unwrap(), 1, "prompt should fire once");
        // Second call: pin matches → no prompt.
        assert!(verifier
            .verify_async("u.example.com", 22, &key)
            .await
            .unwrap());
        assert_eq!(*calls.lock().unwrap(), 1);

        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn verify_async_user_reject_blocks_unknown_host() {
        let path = fresh_tmp_path("async-reject-unknown");
        let _ = fs::remove_file(&path);
        let key = make_test_key(6);

        let prompt: HostKeyPromptCb = Arc::new(|_req| {
            Box::pin(async move { HostKeyDecision::Reject })
        });
        let verifier = HostKeyVerifier::open_ssh_known_hosts(path.clone()).with_prompt(prompt);
        let err = verifier
            .verify_async("r.example.com", 22, &key)
            .await
            .expect_err("rejection must surface as Err");
        assert!(matches!(
            err,
            VerifyError::UserRejected {
                kind: HostKeyPromptKind::Unknown,
                ..
            }
        ));
        assert!(
            !path.exists() || std::fs::read_to_string(&path).unwrap_or_default().is_empty(),
            "rejected host must not be learned",
        );
        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn verify_async_accept_replaces_changed_pin() {
        let path = fresh_tmp_path("async-replace");
        let _ = fs::remove_file(&path);
        let key_a = make_test_key(7);
        let key_b = make_test_key(8);

        // Seed the file with key_a via the no-prompt path.
        let seed = HostKeyVerifier::open_ssh_known_hosts(path.clone());
        assert!(seed.verify("c.example.com", 22, &key_a).unwrap());

        let prompt_kind = Arc::new(std::sync::Mutex::new(None::<HostKeyPromptKind>));
        let prompt_kind_cb = prompt_kind.clone();
        let prompt: HostKeyPromptCb = Arc::new(move |req| {
            *prompt_kind_cb.lock().unwrap() = Some(req.kind);
            Box::pin(async move { HostKeyDecision::Accept })
        });
        let verifier = HostKeyVerifier::open_ssh_known_hosts(path.clone()).with_prompt(prompt);

        // key_b for the same host → user accepts → pin replaced.
        assert!(verifier
            .verify_async("c.example.com", 22, &key_b)
            .await
            .unwrap());
        assert_eq!(
            *prompt_kind.lock().unwrap(),
            Some(HostKeyPromptKind::Changed),
        );
        // Subsequent verify with key_b matches the (now-replaced)
        // pin, no prompt.
        let calls = Arc::new(std::sync::Mutex::new(0u32));
        let calls_cb = calls.clone();
        let prompt2: HostKeyPromptCb = Arc::new(move |_| {
            *calls_cb.lock().unwrap() += 1;
            Box::pin(async move { HostKeyDecision::Reject })
        });
        let verifier2 =
            HostKeyVerifier::open_ssh_known_hosts(path.clone()).with_prompt(prompt2);
        assert!(verifier2
            .verify_async("c.example.com", 22, &key_b)
            .await
            .unwrap());
        assert_eq!(*calls.lock().unwrap(), 0, "matching pin must skip prompt");

        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn verify_async_reject_keeps_old_pin_on_mismatch() {
        let path = fresh_tmp_path("async-reject-changed");
        let _ = fs::remove_file(&path);
        let key_a = make_test_key(9);
        let key_b = make_test_key(10);

        let seed = HostKeyVerifier::open_ssh_known_hosts(path.clone());
        assert!(seed.verify("k.example.com", 22, &key_a).unwrap());
        let original = std::fs::read_to_string(&path).unwrap();

        let prompt: HostKeyPromptCb = Arc::new(|_| {
            Box::pin(async move { HostKeyDecision::Reject })
        });
        let verifier = HostKeyVerifier::open_ssh_known_hosts(path.clone()).with_prompt(prompt);
        let err = verifier
            .verify_async("k.example.com", 22, &key_b)
            .await
            .expect_err("rejection on mismatch must surface as Err");
        assert!(matches!(err, VerifyError::Mismatch { .. }));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "rejected mismatch must not touch the file",
        );
        let _ = fs::remove_file(&path);
    }
}
