//! SSH session — connect, authenticate, open channels.
//!
//! [`SshSession`] wraps a `russh::client::Handle` and provides
//! high-level operations the rest of pier-core actually uses:
//! opening an interactive shell channel, running a one-shot
//! remote command via `exec`, opening a TCP tunnel for a later
//! database-client milestone.
//!
//! ## Sync ↔ async bridge
//!
//! russh is pervasively async. Every method in this module has
//! two forms:
//!
//!  * An `async fn` that returns a `Future` you can `.await` if
//!    your caller already lives inside a tokio context.
//!  * A `*_blocking` variant that enters the process-wide shared
//!    runtime and `block_on`s the async form. These are what the
//!    UI-layer code calls — it is fully synchronous and does not
//!    know or care about tokio.
//!
//! The rule is: if you're already inside a tokio context (e.g.
//! writing a future to be spawned on [`super::runtime::shared`]),
//! call the `async fn`. If you're in plain sync code, call
//! `_blocking`. Never call `_blocking` from within a task that
//! already lives on the shared runtime — that would deadlock the
//! worker thread on `block_on` re-entry.

use std::sync::Arc;
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::ssh_key::PublicKey;

use super::channel::SshChannelPty;
use super::config::{AuthMethod, SshConfig};
use super::error::{Result, SshError};
use super::known_hosts::HostKeyVerifier;
use super::runtime;

/// A live SSH session. Cheap to clone — the underlying
/// `russh::client::Handle` is internally reference-counted, so
/// cloning yields a second pointer to the same connection.
///
/// Drop the last clone to close the connection.
#[derive(Clone)]
pub struct SshSession {
    handle: Arc<Handle<ClientHandler>>,
}

// Manual Debug — the russh Handle itself isn't Debug, and even
// if it were its guts aren't useful to print. We just report
// the refcount so tests + logs can tell whether a session is
// still held by multiple owners.
impl std::fmt::Debug for SshSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshSession")
            .field("handle_refcount", &Arc::strong_count(&self.handle))
            .finish()
    }
}

impl SshSession {
    /// Establish a new SSH connection and authenticate.
    ///
    /// The sequence is: resolve address → TCP connect (with the
    /// timeout from [`SshConfig::connect_timeout_secs`]) → SSH
    /// handshake → host key verification → authentication. If any
    /// of those fails, returns the corresponding [`SshError`]
    /// variant and never hands back a live handle.
    pub async fn connect(config: &SshConfig, verifier: HostKeyVerifier) -> Result<Self> {
        if !config.is_valid() {
            return Err(SshError::InvalidConfig(
                "host, user, port and auth must all be set".to_string(),
            ));
        }

        let russh_config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(300)),
            keepalive_interval: Some(Duration::from_secs(30)),
            ..Default::default()
        });

        // A shared slot the host-key handler writes into when
        // it rejects. We read it back after the handshake
        // finishes (success or failure) to translate an opaque
        // russh protocol error into a structured
        // SshError::HostKeyMismatch.
        let verify_error_slot = std::sync::Arc::new(std::sync::Mutex::new(None));

        let handler = ClientHandler {
            host: config.host.clone(),
            port: config.port,
            verifier,
            last_verify_error: std::sync::Arc::clone(&verify_error_slot),
        };

        let addr = config.address();
        let connect_fut = client::connect(russh_config, addr.clone(), handler);

        // Apply the user-configured connect timeout. `0` = OS
        // default (= whatever russh's internal default does).
        let connect_result = if config.connect_timeout_secs > 0 {
            let timeout = Duration::from_secs(config.connect_timeout_secs);
            match tokio::time::timeout(timeout, connect_fut).await {
                Ok(inner) => inner,
                Err(_) => return Err(SshError::Timeout(timeout)),
            }
        } else {
            connect_fut.await
        };

        let handle = match connect_result {
            Ok(h) => h,
            Err(e) => {
                // If our host-key handler rejected the key,
                // surface the typed error instead of whatever
                // generic "handshake failed" russh produced.
                if let Ok(mut slot) = verify_error_slot.lock() {
                    if let Some(ve) = slot.take() {
                        return Err(verify_error_to_ssh_error(ve));
                    }
                }
                return Err(map_connect_error(e));
            }
        };

        let mut session = Self {
            handle: Arc::new(handle),
        };
        session.authenticate(config).await?;
        Ok(session)
    }

    /// Sync convenience: run [`Self::connect`] on the shared
    /// runtime and block until it completes. Must NOT be called
    /// from inside a task already running on the shared runtime.
    pub fn connect_blocking(config: &SshConfig, verifier: HostKeyVerifier) -> Result<Self> {
        runtime::shared().block_on(Self::connect(config, verifier))
    }

    /// Run every authentication method the config specifies, in
    /// order, until one succeeds. Records which ones we tried so
    /// the [`SshError::AuthRejected`] variant can surface that to
    /// the UI.
    async fn authenticate(&mut self, config: &SshConfig) -> Result<()> {
        let mut tried = Vec::new();

        match &config.auth {
            AuthMethod::InMemoryPassword { password } => {
                tried.push("password (in-memory)".to_string());
                self.try_password_auth(&config.user, password).await?;
            }
            AuthMethod::KeychainPassword { credential_id } => {
                tried.push(format!("password (keychain={credential_id})"));
                // Look the password up from the OS keyring at
                // connect time. The plaintext only ever lives in
                // this stack frame, never on disk and never on
                // the SshConfig struct itself.
                let password = match crate::credentials::get(credential_id) {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        return Err(SshError::InvalidConfig(format!(
                            "no keychain entry for credential_id={credential_id}",
                        )));
                    }
                    Err(e) => {
                        return Err(SshError::InvalidConfig(format!(
                            "keychain lookup failed for {credential_id}: {e}",
                        )));
                    }
                };
                self.try_password_auth(&config.user, &password).await?;
            }
            AuthMethod::PublicKeyFile {
                private_key_path,
                passphrase_credential_id,
            } => {
                tried.push(format!("publickey ({private_key_path})"));
                self.try_publickey_auth(
                    &config.user,
                    private_key_path,
                    passphrase_credential_id.as_deref(),
                )
                .await?;
            }
            AuthMethod::Agent => {
                tried.push("agent".to_string());
                self.try_agent_auth(&config.user).await?;
            }
        }

        // We only reach here if a try_password_auth call returned
        // Ok(()) without short-circuiting via the early `return`
        // inside the helper — i.e. authentication succeeded.
        Ok(())
    }

    /// Shared body of both password-based auth methods. Tries the
    /// password against the open SSH session and returns Ok on
    /// success. On rejection, returns the AuthRejected error
    /// stamped with `tried` so the UI can show what we attempted.
    async fn try_password_auth(&mut self, user: &str, password: &str) -> Result<()> {
        // SAFETY: we just Arc::new'd this handle in connect();
        // we're the only holder at this point so get_mut is fine.
        let handle = Arc::get_mut(&mut self.handle).expect("unique handle during auth");
        let ok = handle.authenticate_password(user, password.to_string()).await?;
        if !ok.success() {
            return Err(SshError::AuthRejected {
                tried: vec!["password".to_string()],
            });
        }
        Ok(())
    }

    /// Authenticate via the system SSH agent.
    ///
    /// On Unix we connect to `$SSH_AUTH_SOCK`; on Windows we
    /// use Pageant's named pipe. russh handles both through the
    /// platform-specific `AgentClient::connect_env` / `connect_pageant`
    /// constructors. The agent hands us a list of identities;
    /// we walk them in order and try `authenticate_publickey_with`
    /// (which uses the agent as a `Signer`) until one succeeds.
    async fn try_agent_auth(&mut self, user: &str) -> Result<()> {
        // Grab the handle first; we'll re-grab it inside the
        // loop because authenticate_publickey_with takes &mut.
        // No other holder exists during connect() so get_mut
        // unwraps safely.
        let handle = Arc::get_mut(&mut self.handle).expect("unique handle during auth");

        // Connect to the agent. On Unix, connect_env reads
        // $SSH_AUTH_SOCK. If the variable isn't set or the
        // socket is absent, we surface an InvalidConfig so the
        // UI can say "no agent found" instead of the generic
        // AuthRejected path (which would imply the agent HAD
        // keys and they were rejected).
        #[cfg(unix)]
        let mut agent = match russh::keys::agent::client::AgentClient::connect_env().await {
            Ok(a) => a,
            Err(e) => {
                return Err(SshError::InvalidConfig(format!(
                    "SSH agent not available (SSH_AUTH_SOCK?): {e}",
                )));
            }
        };
        #[cfg(windows)]
        let mut agent = match russh::keys::agent::client::AgentClient::connect_pageant().await {
            Ok(a) => a,
            Err(e) => {
                return Err(SshError::InvalidConfig(format!(
                    "SSH agent (Pageant) not available: {e}",
                )));
            }
        };
        #[cfg(not(any(unix, windows)))]
        {
            let _ = user;
            return Err(SshError::Unsupported);
        }

        let identities = agent
            .request_identities()
            .await
            .map_err(|e| SshError::InvalidConfig(format!("SSH agent list failed: {e}")))?;

        if identities.is_empty() {
            return Err(SshError::AuthRejected {
                tried: vec!["agent (no identities)".to_string()],
            });
        }

        // Walk identities and try each. The first one the
        // server accepts wins. If none do, fail with AuthRejected
        // listing how many we tried.
        let mut attempted = 0usize;
        for identity in &identities {
            attempted += 1;
            let pubkey = identity.public_key().into_owned();
            match handle
                .authenticate_publickey_with(user, pubkey, None, &mut agent)
                .await
            {
                Ok(result) => {
                    if result.success() {
                        return Ok(());
                    }
                }
                Err(e) => {
                    log::warn!("agent auth attempt {attempted} failed: {e}");
                }
            }
        }

        Err(SshError::AuthRejected {
            tried: vec![format!("agent ({attempted} identities rejected)")],
        })
    }

    /// Authenticate via an OpenSSH-format private key file.
    ///
    /// `private_key_path` is the on-disk location of the key
    /// (typically `~/.ssh/id_ed25519`). If the key is encrypted,
    /// `passphrase_credential_id` must reference a keychain
    /// entry holding the passphrase — the same shape used by
    /// [`AuthMethod::KeychainPassword`]. The plaintext
    /// passphrase only ever lives in this stack frame, never on
    /// disk and never on the SshConfig struct.
    async fn try_publickey_auth(
        &mut self,
        user: &str,
        private_key_path: &str,
        passphrase_credential_id: Option<&str>,
    ) -> Result<()> {
        use std::sync::Arc as StdArc;

        // Resolve the passphrase, if any. A missing keychain
        // entry is treated as a fatal config error rather than
        // "no passphrase" — if the user told us to look one up
        // they meant it, and silently falling back to "no
        // passphrase" would surface as a confusing decode error.
        let passphrase: Option<String> = match passphrase_credential_id {
            None => None,
            Some(id) => match crate::credentials::get(id) {
                Ok(Some(p)) => Some(p),
                Ok(None) => {
                    return Err(SshError::InvalidConfig(format!(
                        "no keychain entry for passphrase credential_id={id}",
                    )));
                }
                Err(e) => {
                    return Err(SshError::InvalidConfig(format!(
                        "keychain lookup failed for passphrase {id}: {e}",
                    )));
                }
            },
        };

        let key = russh::keys::load_secret_key(private_key_path, passphrase.as_deref())
            .map_err(|e| {
                SshError::InvalidConfig(format!(
                    "failed to load private key {private_key_path}: {e}",
                ))
            })?;

        let key_with_hash =
            russh::keys::PrivateKeyWithHashAlg::new(StdArc::new(key), None);

        // SAFETY: we just Arc::new'd this handle in connect();
        // we're the only holder at this point so get_mut is fine.
        let handle = Arc::get_mut(&mut self.handle).expect("unique handle during auth");
        let ok = handle
            .authenticate_publickey(user, key_with_hash)
            .await?;
        if !ok.success() {
            return Err(SshError::AuthRejected {
                tried: vec!["publickey".to_string()],
            });
        }
        Ok(())
    }

    /// Open a new interactive shell channel on the remote host
    /// and wrap it in an [`SshChannelPty`] that implements the
    /// [`crate::terminal::Pty`] trait. The returned value can be
    /// handed directly to [`crate::terminal::PierTerminal::with_pty`]
    /// — which is the whole point of the M2 trait design.
    pub async fn open_shell_channel(&self, cols: u16, rows: u16) -> Result<SshChannelPty> {
        let channel = self.handle.channel_open_session().await?;
        // Request a real PTY on the remote so TUIs like vim and
        // htop run correctly. xterm-256color matches what
        // terminal::pty::UnixPty pins for local shells.
        channel
            .request_pty(
                false, // no reply needed
                "xterm-256color",
                cols as u32,
                rows as u32,
                0,
                0,
                &[],
            )
            .await?;
        channel.request_shell(false).await?;
        Ok(SshChannelPty::spawn(channel, cols, rows))
    }

    /// Sync convenience for [`Self::open_shell_channel`].
    pub fn open_shell_channel_blocking(&self, cols: u16, rows: u16) -> Result<SshChannelPty> {
        runtime::shared().block_on(self.open_shell_channel(cols, rows))
    }

    /// Returns the number of strong references still holding this
    /// session alive. Used by tests and by M3b's connection
    /// manager to decide when a session can be closed.
    pub fn handle_refcount(&self) -> usize {
        Arc::strong_count(&self.handle)
    }
}

fn map_connect_error(e: russh::Error) -> SshError {
    // russh wraps the underlying std::io::Error in some variants;
    // unwrap into a Connect error when that's the case so the UI
    // can distinguish "DNS failed" from "auth rejected".
    match e {
        russh::Error::IO(io) => SshError::Connect(io),
        other => SshError::Protocol(other),
    }
}

/// Translate a VerifyError from the host-key verifier into the
/// UI-facing SshError. Mismatches surface as `HostKeyMismatch`
/// so the Failed overlay can show the fingerprint and prompt
/// the user; I/O errors surface as `InvalidConfig` since they
/// indicate an unreadable known_hosts file rather than a bad
/// connection.
fn verify_error_to_ssh_error(e: super::known_hosts::VerifyError) -> SshError {
    use super::known_hosts::VerifyError;
    match e {
        VerifyError::Mismatch { host, fingerprint, .. } => {
            SshError::HostKeyMismatch { host, fingerprint }
        }
        VerifyError::Io(msg) => SshError::InvalidConfig(format!("known_hosts: {msg}")),
    }
}

/// russh's callback surface for a client-side connection.
///
/// Host key verification lives inside `check_server_key` — we
/// delegate to the [`HostKeyVerifier`] the session was constructed
/// with so the swap from the M3a accept-all verifier to the M3c4
/// real known_hosts verifier is a single call-site change.
///
/// The `last_verify_error` slot captures structured mismatch
/// details from inside the async handler — russh's
/// `check_server_key` can only return `Ok(bool)`, so we record
/// the real reason here and `SshSession::connect` reads it back
/// after the handshake fails to translate into a typed
/// [`SshError::HostKeyMismatch`].
pub struct ClientHandler {
    host: String,
    port: u16,
    verifier: HostKeyVerifier,
    last_verify_error: std::sync::Arc<std::sync::Mutex<Option<super::known_hosts::VerifyError>>>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> std::result::Result<bool, Self::Error> {
        match self.verifier.verify(&self.host, self.port, server_public_key) {
            Ok(accept) => Ok(accept),
            Err(e) => {
                log::warn!(
                    "host key verification failed for {}:{}: {e}",
                    self.host, self.port,
                );
                // Stash the structured error so SshSession::connect
                // can translate it into SshError::HostKeyMismatch
                // / SshError::InvalidConfig after the handshake
                // unwinds. Poisoned mutex is a test-only concern
                // in practice; just unwrap.
                if let Ok(mut slot) = self.last_verify_error.lock() {
                    *slot = Some(e);
                }
                // Return false — russh treats this as a rejected
                // handshake and propagates up a protocol error.
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_blocking_rejects_invalid_config() {
        // Empty host → InvalidConfig, never touches the network.
        let cfg = SshConfig::new("test", "", "");
        let err = SshSession::connect_blocking(&cfg, HostKeyVerifier::default())
            .expect_err("invalid config must be rejected before dialing");
        assert!(
            matches!(err, SshError::InvalidConfig(_)),
            "expected InvalidConfig, got {err:?}",
        );
    }

    #[test]
    fn connect_blocking_times_out_on_unreachable_host() {
        // RFC 5737 TEST-NET-1 — guaranteed unroutable.
        // This test hits the timeout branch of connect() without
        // depending on any DNS lookup or real network state.
        let mut cfg = SshConfig::new("test", "192.0.2.1", "root");
        cfg.auth = AuthMethod::InMemoryPassword {
            password: "x".into(),
        };
        cfg.connect_timeout_secs = 1;

        let start = std::time::Instant::now();
        let err = SshSession::connect_blocking(&cfg, HostKeyVerifier::default())
            .expect_err("unreachable host must fail");
        let elapsed = start.elapsed();

        // Must fail in under ~3 seconds (1s configured + slop).
        assert!(
            elapsed < Duration::from_secs(5),
            "connect should respect the 1s timeout but took {elapsed:?}",
        );
        // Either Timeout (expected on most setups) or Connect
        // (if the OS returns ECONNREFUSED / EHOSTUNREACH fast
        // enough). Both are acceptable — what we're asserting
        // here is "it fails fast, with a typed error, and
        // doesn't panic or hang".
        assert!(
            matches!(err, SshError::Timeout(_) | SshError::Connect(_) | SshError::Protocol(_)),
            "expected Timeout / Connect / Protocol, got {err:?}",
        );
    }
}
