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
use tokio_util::sync::CancellationToken;

use super::channel::SshChannelPty;
use super::config::{AuthMethod, SshConfig};
use super::error::{Result, SshError};
use super::known_hosts::HostKeyVerifier;
use super::runtime;

/// Sentinel exit code returned by [`SshSession::exec_command_streaming`]
/// when a caller-supplied [`CancellationToken`] fires before the remote
/// command produced its own exit status. Distinct from `-1` (channel
/// closed without an `ExitStatus`) and from any real shell exit code so
/// callers — most importantly `services::package_manager` — can branch
/// on "user cancelled" without misreading a remote `exit 254` as a
/// cancellation.
pub const CANCELLED_EXIT_CODE: i32 = -2;

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

        // Apply the same connect timeout to authentication. Without
        // it, an unresponsive agent (Windows OpenSSH agent that's
        // started but not answering) or a remote sshd that accepts
        // the kex but stalls during userauth — both of which can
        // happen on a hosts under load — leaves us blocked forever
        // and the UI shows "Launching shell..." with no recourse.
        if config.connect_timeout_secs > 0 {
            let timeout = Duration::from_secs(config.connect_timeout_secs);
            match tokio::time::timeout(timeout, session.authenticate(config)).await {
                Ok(inner) => inner?,
                Err(_) => return Err(SshError::Timeout(timeout)),
            }
        } else {
            session.authenticate(config).await?;
        }
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
            AuthMethod::DirectPassword { password } => {
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
                            "saved password missing in keychain (credential_id={credential_id})",
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
            AuthMethod::Auto => {
                // OpenSSH-style "just try things": agent first, then
                // each conventional default identity file that
                // actually exists on disk. We chain attempts on the
                // SAME session so the cost is one kex + N auth
                // attempts, not N full handshakes.
                //
                // Ordering rationale: agent wins when it's running
                // (it's what the terminal-side ssh.exe most often
                // used on Windows via the OpenSSH agent service);
                // otherwise we fall through to the default key
                // filenames OpenSSH itself probes in the same order.
                // Encrypted keys without a keychain passphrase will
                // fail silently and fall through — the user would
                // see them prompt in a real `ssh` invocation, which
                // we can't replicate here.
                let mut any_success = false;
                match self.try_agent_auth(&config.user).await {
                    Ok(()) => {
                        tried.push("agent".to_string());
                        any_success = true;
                    }
                    Err(e) => {
                        tried.push(format!("agent ({e})"));
                    }
                }

                if !any_success {
                    for key_path in default_identity_paths() {
                        if !std::path::Path::new(&key_path).is_file() {
                            continue;
                        }
                        tried.push(format!("publickey ({key_path})"));
                        match self.try_publickey_auth(&config.user, &key_path, None).await {
                            Ok(()) => {
                                any_success = true;
                                break;
                            }
                            Err(e) => {
                                // Log and keep walking — a missing
                                // passphrase on one key shouldn't
                                // block the others.
                                log::debug!("auto auth: {key_path} rejected: {e}");
                            }
                        }
                    }
                }

                if !any_success {
                    return Err(SshError::AuthRejected { tried });
                }
            }
            AuthMethod::AutoChain {
                explicit_key_path,
                password,
                key_passphrase,
            } => {
                // Same one-transport-many-attempts shape as Auto, but
                // adds explicit key / password / keyboard-interactive
                // legs after the agent + default-keys probe. Intent:
                // when the watcher saw a plain `ssh user@host` it
                // can't tell upfront whether the server wants
                // pubkey, password, or PAM — try every method we
                // have material for, in OpenSSH-like preference, on
                // a single SSH session.
                let mut any_success = false;

                // 1. Agent — wins immediately on the common laptop
                //    setup (ssh-agent / Pageant holding loaded keys).
                match self.try_agent_auth(&config.user).await {
                    Ok(()) => {
                        tried.push("agent".to_string());
                        any_success = true;
                    }
                    Err(e) => {
                        tried.push(format!("agent ({e})"));
                    }
                }

                // 2. Explicit `-i <path>` from the terminal, if any.
                //    Try with the captured passphrase first (if we
                //    have one); ssh's load_secret_key fails fast on
                //    a wrong passphrase, so the cost is bounded.
                if !any_success {
                    if let Some(path) = explicit_key_path
                        .as_deref()
                        .filter(|p| !p.is_empty() && std::path::Path::new(p).is_file())
                    {
                        tried.push(format!("publickey ({path})"));
                        match self
                            .try_publickey_auth_raw(&config.user, path, key_passphrase.as_deref())
                            .await
                        {
                            Ok(()) => {
                                any_success = true;
                            }
                            Err(e) => {
                                log::debug!("autochain explicit key {path} rejected: {e}");
                            }
                        }
                    }
                }

                // 3. Conventional default identity files — same set
                //    OpenSSH probes, in the same order. Skip the one
                //    we already tried as `explicit_key_path`. The
                //    passphrase is fed to every attempt; an
                //    unencrypted key whose loader rejects an
                //    unsolicited passphrase just falls through to
                //    the next chain leg. We tolerate the false-
                //    negative because the alternative (try-without
                //    then try-with) doubles the work for the common
                //    case where the user's keys all share one
                //    passphrase or no passphrase.
                if !any_success {
                    let already_tried = explicit_key_path.as_deref().unwrap_or("");
                    for key_path in default_identity_paths() {
                        if !std::path::Path::new(&key_path).is_file() {
                            continue;
                        }
                        if key_path == already_tried {
                            continue;
                        }
                        tried.push(format!("publickey ({key_path})"));
                        match self
                            .try_publickey_auth_raw(
                                &config.user,
                                &key_path,
                                key_passphrase.as_deref(),
                            )
                            .await
                        {
                            Ok(()) => {
                                any_success = true;
                                break;
                            }
                            Err(e) => {
                                log::debug!("autochain default key {key_path} rejected: {e}");
                            }
                        }
                    }
                }

                // 4. Password — captured from the OpenSSH prompt the
                //    user just answered successfully in their `ssh`
                //    child. Servers configured for plain "password"
                //    auth land here.
                if !any_success {
                    if let Some(pw) = password.as_deref().filter(|s| !s.is_empty()) {
                        tried.push("password".to_string());
                        match self.try_password_auth(&config.user, pw).await {
                            Ok(()) => {
                                any_success = true;
                            }
                            Err(e) => {
                                log::debug!("autochain password rejected: {e}");
                            }
                        }
                    }
                }

                // 5. Keyboard-interactive — most PAM-backed servers
                //    (sshd's default on many distros) advertise this
                //    method instead of plain "password". Single-prompt
                //    challenges accept the captured password as the
                //    response; multi-prompt MFA stacks won't survive
                //    this leg, but neither would any of the others,
                //    and chaining costs nothing on the same transport.
                if !any_success {
                    if let Some(pw) = password.as_deref().filter(|s| !s.is_empty()) {
                        tried.push("keyboard-interactive".to_string());
                        match self
                            .try_keyboard_interactive_with_password(&config.user, pw)
                            .await
                        {
                            Ok(()) => {
                                any_success = true;
                            }
                            Err(e) => {
                                log::debug!("autochain keyboard-interactive rejected: {e}");
                            }
                        }
                    }
                }

                if !any_success {
                    return Err(SshError::AuthRejected { tried });
                }
            }
        }

        // We only reach here if a try_password_auth call returned
        // Ok(()) without short-circuiting via the early `return`
        // inside the helper — i.e. authentication succeeded.
        Ok(())
    }

    /// Attempt keyboard-interactive auth, answering every prompt the
    /// server sends with `password`. PAM-backed sshd configurations
    /// typically advertise this method (rather than plain "password")
    /// for password challenges, and a single-prompt PAM stack is
    /// indistinguishable from an interactive password from the user's
    /// perspective — so we hand back the same string they just typed
    /// at the OpenSSH prompt.
    ///
    /// Bounded loop: russh allows the server to send any number of
    /// `InfoRequest` rounds; we cap at 4 so a misconfigured /
    /// adversarial server can't pin us in an infinite ping-pong.
    async fn try_keyboard_interactive_with_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<()> {
        use russh::client::KeyboardInteractiveAuthResponse;

        // SAFETY: same invariant as the other `try_*_auth` helpers —
        // during connect() we are the only Arc holder.
        let handle = Arc::get_mut(&mut self.handle).expect("unique handle during auth");

        let mut response = handle
            .authenticate_keyboard_interactive_start(user, None::<String>)
            .await?;

        for _ in 0..4 {
            match response {
                KeyboardInteractiveAuthResponse::Success => return Ok(()),
                KeyboardInteractiveAuthResponse::Failure { .. } => {
                    return Err(SshError::AuthRejected {
                        tried: vec!["keyboard-interactive".to_string()],
                    });
                }
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let answers: Vec<String> =
                        prompts.iter().map(|_| password.to_string()).collect();
                    response = handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await?;
                }
            }
        }

        Err(SshError::AuthRejected {
            tried: vec!["keyboard-interactive (prompt loop exceeded)".to_string()],
        })
    }

    /// Shared body of both password-based auth methods. Tries the
    /// password against the open SSH session and returns Ok on
    /// success. On rejection, returns the AuthRejected error
    /// stamped with `tried` so the UI can show what we attempted.
    async fn try_password_auth(&mut self, user: &str, password: &str) -> Result<()> {
        // SAFETY: we just Arc::new'd this handle in connect();
        // we're the only holder at this point so get_mut is fine.
        let handle = Arc::get_mut(&mut self.handle).expect("unique handle during auth");
        let ok = handle
            .authenticate_password(user, password.to_string())
            .await?;
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
        // Resolve the passphrase from the keychain, if asked. A
        // missing keychain entry is treated as a fatal config error
        // rather than "no passphrase" — if the user told us to look
        // one up they meant it, and silently falling back to "no
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
        self.try_publickey_auth_raw(user, private_key_path, passphrase.as_deref())
            .await
    }

    /// Like [`Self::try_publickey_auth`] but takes the passphrase as
    /// a literal string instead of a keychain credential ID. Used by
    /// AutoChain to feed in the passphrase the user just typed at
    /// the terminal-side ssh prompt — that one isn't in the
    /// keychain, it's in our process-level credential cache.
    ///
    /// `passphrase=None` means "key isn't encrypted" (load_secret_key
    /// will succeed if so, fail otherwise — same as omitting `-N`
    /// to ssh-keygen).
    async fn try_publickey_auth_raw(
        &mut self,
        user: &str,
        private_key_path: &str,
        passphrase: Option<&str>,
    ) -> Result<()> {
        use std::sync::Arc as StdArc;

        let key = russh::keys::load_secret_key(private_key_path, passphrase).map_err(|e| {
            SshError::InvalidConfig(format!(
                "failed to load private key {private_key_path}: {e}",
            ))
        })?;

        let key_with_hash = russh::keys::PrivateKeyWithHashAlg::new(StdArc::new(key), None);

        // SAFETY: we just Arc::new'd this handle in connect();
        // we're the only holder at this point so get_mut is fine.
        let handle = Arc::get_mut(&mut self.handle).expect("unique handle during auth");
        let ok = handle.authenticate_publickey(user, key_with_hash).await?;
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

    /// Open an SFTP subsystem channel on this session.
    ///
    /// Internally this opens a fresh channel, calls
    /// `request_subsystem("sftp")`, and hands the channel
    /// stream to russh-sftp's `SftpSession::new`. The
    /// resulting [`super::SftpClient`] shares the underlying
    /// SSH connection with whatever terminal channels are
    /// already open — no additional TCP connection, no
    /// additional authentication.
    ///
    /// One `SftpClient` per UI panel is the expected usage.
    /// Multiple SFTP clients on the same session are allowed
    /// (each opens its own channel) but share the session's
    /// bandwidth.
    pub async fn open_sftp(&self) -> Result<super::SftpClient> {
        let channel = self.handle.channel_open_session().await?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| SshError::InvalidConfig(format!("request_subsystem(sftp): {e}")))?;
        let session = russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| SshError::InvalidConfig(format!("SftpSession::new: {e}")))?;
        Ok(super::SftpClient::new(session))
    }

    /// Sync convenience for [`Self::open_sftp`].
    pub fn open_sftp_blocking(&self) -> Result<super::SftpClient> {
        runtime::shared().block_on(self.open_sftp())
    }

    /// Run `command` remotely via SSH exec, wait for it to
    /// finish, and return `(exit_status, stdout)`. Stderr is
    /// intentionally dropped — the service_detector and its
    /// siblings only care about stdout + the exit code. A
    /// future variant can return `(code, stdout, stderr)`
    /// when a caller actually needs it.
    ///
    /// The returned string is UTF-8; non-UTF-8 output bytes
    /// are replaced via `String::from_utf8_lossy` so the
    /// service_detector's substring-based matching never hits
    /// a decode error from a mis-tagged binary on the remote.
    pub async fn exec_command(&self, command: &str) -> Result<(i32, String)> {
        self.exec_command_streaming(command, |_| {}, None).await
    }

    /// Sync convenience for [`Self::exec_command`].
    pub fn exec_command_blocking(&self, command: &str) -> Result<(i32, String)> {
        runtime::shared().block_on(self.exec_command(command))
    }

    /// Run `command` remotely and stream every complete output line
    /// (stdout *and* stderr, merged in arrival order) through `on_line`
    /// while it runs. Returns the same `(exit_status, full_stdout)` as
    /// [`Self::exec_command`] when the channel closes — `full_stdout`
    /// here contains the same merged text the callback saw.
    ///
    /// Lines are emitted as `\n`-delimited UTF-8; trailing `\r` (CRLF
    /// hosts) is stripped before the callback fires. A partial trailing
    /// line (no `\n`) is flushed as a final callback after the channel
    /// closes, so progress shells like `apt-get -qq` whose final line
    /// lacks a newline still surface to the UI.
    ///
    /// Long-running installs (`apt-get update && install`, `docker pull`)
    /// rely on this so the panel can render progress instead of waiting
    /// 30+ seconds for one final dump.
    ///
    /// `cancel = Some(token)` makes the loop cancellable: the moment the
    /// token fires we close the channel and return early with
    /// `exit_code = ` [`CANCELLED_EXIT_CODE`] (`-2`). `cancel = None`
    /// preserves the pre-cancellation behaviour for every existing
    /// caller (no select overhead, no behavioural change).
    ///
    /// Note that a successful cancel only stops *us* from reading more
    /// output — the remote process is still alive. Package-manager
    /// front-ends like `apt-get install` may leave a `dpkg` lock held;
    /// the user is expected to clean that up. This trade-off is
    /// documented in the Software panel's PRODUCT-SPEC §5.11 v2 note.
    pub async fn exec_command_streaming<F>(
        &self,
        command: &str,
        mut on_line: F,
        cancel: Option<CancellationToken>,
    ) -> Result<(i32, String)>
    where
        F: FnMut(&str),
    {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;

        let mut full = Vec::new();
        let mut line_buf: Vec<u8> = Vec::new();
        let mut exit_code: i32 = -1;
        let mut cancelled = false;
        loop {
            // Race the remote message stream against the cancellation
            // token when one was supplied. `biased;` forces the cancel
            // branch to be polled first — without it, a busy stream of
            // Data frames could keep us scheduled on `channel.wait()`
            // and starve the cancel arm. With `cancel = None` we take
            // the no-overhead fast path.
            let next = match &cancel {
                Some(tok) => {
                    tokio::select! {
                        biased;
                        _ = tok.cancelled() => {
                            cancelled = true;
                            None
                        }
                        msg = channel.wait() => msg,
                    }
                }
                None => channel.wait().await,
            };
            if cancelled {
                break;
            }
            let Some(msg) = next else {
                // Channel closed without an ExitStatus — keep whatever
                // we accumulated; callers check exit_code.
                break;
            };
            match msg {
                russh::ChannelMsg::Data { data } => {
                    Self::drain_chunk(&data, &mut full, &mut line_buf, &mut on_line);
                }
                russh::ChannelMsg::ExtendedData { data, ext: _ } => {
                    // Merge stderr — install commands print warnings and
                    // package-manager prompts on stderr that the UI
                    // absolutely needs to see.
                    Self::drain_chunk(&data, &mut full, &mut line_buf, &mut on_line);
                }
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = exit_status as i32;
                }
                russh::ChannelMsg::Eof | russh::ChannelMsg::Close => {}
                _ => {}
            }
        }
        if cancelled {
            // Best-effort: tell the server we're done with this channel.
            // We don't wait for confirmation — the user wanted out fast.
            let _ = channel.close().await;
            exit_code = CANCELLED_EXIT_CODE;
        }
        if !line_buf.is_empty() {
            let trimmed = line_buf
                .strip_suffix(b"\r")
                .unwrap_or(&line_buf);
            on_line(&String::from_utf8_lossy(trimmed));
        }
        Ok((exit_code, String::from_utf8_lossy(&full).into_owned()))
    }

    /// Sync convenience for [`Self::exec_command_streaming`].
    pub fn exec_command_streaming_blocking<F>(
        &self,
        command: &str,
        on_line: F,
        cancel: Option<CancellationToken>,
    ) -> Result<(i32, String)>
    where
        F: FnMut(&str),
    {
        runtime::shared().block_on(self.exec_command_streaming(command, on_line, cancel))
    }

    /// Append `chunk` to the running buffer and emit any complete
    /// `\n`-terminated lines through `on_line`. Partial trailing line
    /// stays in `line_buf` until the next chunk arrives. Splits on
    /// raw bytes — multi-byte UTF-8 sequences split across chunks are
    /// safe because we only look at the `\n` byte.
    fn drain_chunk<F: FnMut(&str)>(
        chunk: &[u8],
        full: &mut Vec<u8>,
        line_buf: &mut Vec<u8>,
        on_line: &mut F,
    ) {
        full.extend_from_slice(chunk);
        line_buf.extend_from_slice(chunk);
        while let Some(nl) = line_buf.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = line_buf.drain(..=nl).collect();
            line.pop(); // drop the \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            on_line(&String::from_utf8_lossy(&line));
        }
    }

    /// Returns the number of strong references still holding this
    /// session alive. Used by tests and by M3b's connection
    /// manager to decide when a session can be closed.
    pub fn handle_refcount(&self) -> usize {
        Arc::strong_count(&self.handle)
    }

    /// Internal: expose the underlying russh handle Arc so
    /// sibling modules (notably [`super::tunnel`]) can hold
    /// it across task boundaries without smuggling the Arc
    /// through a private field. Kept `pub(super)` because the
    /// russh `Handle<ClientHandler>` type is an implementation
    /// detail — nothing outside the `ssh` module should depend
    /// on its shape.
    pub(super) fn handle_arc(&self) -> Arc<Handle<ClientHandler>> {
        Arc::clone(&self.handle)
    }
}

/// Conventional default identity filenames OpenSSH probes when no
/// `-i` / `IdentityFile` is configured. Order matches `ssh(1)`'s
/// man page so a user with a mix of key types sees the same
/// preference as their terminal client. Non-existent paths are
/// skipped by the caller.
fn default_identity_paths() -> Vec<String> {
    let Some(home) = dirs_home() else {
        return Vec::new();
    };
    let base = home.join(".ssh");
    let names = [
        "id_ed25519",
        "id_ecdsa",
        "id_ecdsa_sk",
        "id_ed25519_sk",
        "id_rsa",
        "id_dsa",
    ];
    names
        .iter()
        .map(|n| base.join(n).to_string_lossy().into_owned())
        .collect()
}

/// Resolve the user's home directory without bringing in a new crate.
/// `directories` is already a pier-core dep but only exposes
/// project/data dirs; the `$HOME` / `%USERPROFILE%` fallback below is
/// the simplest version that works on all three desktop targets.
fn dirs_home() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        if let Ok(v) = std::env::var("USERPROFILE") {
            if !v.is_empty() {
                return Some(std::path::PathBuf::from(v));
            }
        }
    }
    if let Ok(v) = std::env::var("HOME") {
        if !v.is_empty() {
            return Some(std::path::PathBuf::from(v));
        }
    }
    None
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
        VerifyError::Mismatch {
            host, fingerprint, ..
        } => SshError::HostKeyMismatch { host, fingerprint },
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
        match self
            .verifier
            .verify(&self.host, self.port, server_public_key)
        {
            Ok(accept) => Ok(accept),
            Err(e) => {
                log::warn!(
                    "host key verification failed for {}:{}: {e}",
                    self.host,
                    self.port,
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
        cfg.auth = AuthMethod::DirectPassword {
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
            matches!(
                err,
                SshError::Timeout(_) | SshError::Connect(_) | SshError::Protocol(_)
            ),
            "expected Timeout / Connect / Protocol, got {err:?}",
        );
    }

    fn drain_capture(
        input: &[u8],
        full: &mut Vec<u8>,
        buf: &mut Vec<u8>,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        SshSession::drain_chunk(input, full, buf, &mut |s: &str| {
            lines.push(s.to_string())
        });
        lines
    }

    #[test]
    fn drain_chunk_emits_complete_lines_only() {
        let mut full = Vec::new();
        let mut buf = Vec::new();

        assert_eq!(drain_capture(b"hello\nworld", &mut full, &mut buf), vec!["hello"]);
        assert_eq!(buf, b"world");

        assert_eq!(drain_capture(b"\nlast", &mut full, &mut buf), vec!["world"]);
        assert_eq!(buf, b"last");

        // Trailing line without \n stays in buf until the caller flushes.
        assert_eq!(drain_capture(b"\n", &mut full, &mut buf), vec!["last"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_chunk_strips_trailing_cr() {
        let mut full = Vec::new();
        let mut buf = Vec::new();
        assert_eq!(
            drain_capture(b"crlf\r\nlf\n", &mut full, &mut buf),
            vec!["crlf", "lf"],
        );
    }

    /// Exercises the same `tokio::select!` shape used by
    /// [`SshSession::exec_command_streaming`] when a cancellation token
    /// is supplied. We can't reach the real method without an SSH
    /// server, so this asserts the cancellable wait pattern itself —
    /// when the token fires, the future stops within ~tens of ms and
    /// surfaces a "cancelled" outcome the caller can map to
    /// [`CANCELLED_EXIT_CODE`].
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancellable_wait_pattern_returns_within_100ms() {
        let token = CancellationToken::new();
        let trigger = token.clone();

        // Fire the cancel after 20ms — well inside the 100ms budget the
        // package-manager spec promises the user.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            trigger.cancel();
        });

        let start = std::time::Instant::now();
        // Stand-in for `channel.wait()` — a future that would normally
        // stay pending for a long time. The select! inside the real
        // implementation drops it the instant the token fires.
        let fake_wait = tokio::time::sleep(Duration::from_secs(60));
        let outcome = tokio::select! {
            biased;
            _ = token.cancelled() => CANCELLED_EXIT_CODE,
            _ = fake_wait => 0,
        };
        let elapsed = start.elapsed();

        assert_eq!(outcome, CANCELLED_EXIT_CODE);
        assert!(
            elapsed < Duration::from_millis(100),
            "cancel should observe within 100ms but took {elapsed:?}",
        );
    }

    #[test]
    fn drain_chunk_handles_split_utf8_across_chunks() {
        // The Chinese character "中" is 3 bytes (0xE4 0xB8 0xAD).
        // Splitting between bytes must not panic — we only look at \n.
        let mut full = Vec::new();
        let mut buf = Vec::new();

        assert!(
            drain_capture(&[0xE4, 0xB8], &mut full, &mut buf).is_empty(),
            "no \\n yet, no emission",
        );
        assert_eq!(
            drain_capture(&[0xAD, b'\n'], &mut full, &mut buf),
            vec!["中".to_string()],
        );
    }
}
