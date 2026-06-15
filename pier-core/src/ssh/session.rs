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
use tokio::sync::RwLock;
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
/// Dropping the last clone closes the connection only once every
/// channel opened from it is gone too: each live `russh` channel
/// holds its own sender clone into the transport task, so open
/// PTYs / execs keep the transport alive after the last
/// `SshSession` drops. Cache eviction in the Tauri layer relies on
/// this — evicting a session never yanks the connection out from
/// under sibling tabs' channels.
#[derive(Clone)]
pub struct SshSession {
    handle: Arc<Handle<ClientHandler>>,
    /// Optional sudo / privilege-escalation password attached to
    /// this session by the Tauri layer when a panel needs to run
    /// commands as root (Docker daemon, firewall, nginx config…).
    /// When `Some`, [`Self::exec_with_sudo`] wraps each command in
    /// `sudo -S -p ''` and pipes the password via stdin; when
    /// `None`, [`Self::exec_with_sudo`] degrades to a plain exec.
    /// Per-session storage means a single host's panels share one
    /// password without re-prompting; different SSH users still
    /// get distinct sessions and therefore distinct slots.
    sudo_password: Arc<RwLock<Option<String>>>,
    /// Privilege-escalation method paired with `sudo_password` for
    /// [`Self::exec_with_sudo`]. Defaults to [`Elevation::Sudo`] so the
    /// legacy "password set → `sudo -S` as root" behavior is preserved;
    /// the Tauri layer overrides it via [`Self::set_elevation`] to follow
    /// the terminal's effective user (e.g. `sudo -u deploy` after the
    /// operator `su - deploy`'d in the terminal). `exec_with_sudo` runs
    /// `exec_as_effective(cmd, &this, password)`.
    elevation: Arc<RwLock<crate::sudo::Elevation>>,
    /// Whether this host is known to be elevated in the terminal even
    /// though no secret was captured — e.g. the operator ran `sudo -i` on
    /// a NOPASSWD / cached-credentials host, so there was no password
    /// prompt to capture. When `true` and `sudo_password` is empty,
    /// [`Self::exec_with_sudo`] attempts a passwordless `sudo -n` and
    /// degrades to an unprivileged run if the host actually needs a
    /// password (rather than failing the operation). Set by the Tauri
    /// layer from the terminal's observed effective user.
    elevation_armed: Arc<RwLock<bool>>,
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
        Self::connect_with_egress(config, verifier, None).await
    }

    /// Sync convenience: run [`Self::connect`] on the shared
    /// runtime and block until it completes. Must NOT be called
    /// from inside a task already running on the shared runtime.
    pub fn connect_blocking(config: &SshConfig, verifier: HostKeyVerifier) -> Result<Self> {
        runtime::shared().block_on(Self::connect(config, verifier))
    }

    /// Whether the underlying russh transport task has stopped — i.e. the
    /// connection is dead (TCP reset, server disconnect, keepalive
    /// timeout). The Tauri cache layer uses this to tell a genuinely dead
    /// session (evict + reconnect) apart from a session that's alive but
    /// where the *operation* failed (a command returned non-zero, a path
    /// was missing); only the former should tear down the shared
    /// connection. See `run_with_session_retry`.
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Same contract as [`Self::connect`], but routes the underlying
    /// TCP transport through `egress` when supplied. `egress = None`
    /// is exactly equivalent to [`Self::connect`].
    ///
    /// When an egress profile is given, [`crate::egress::resolve_tcp`]
    /// produces the byte stream the SSH handshake runs on top of.
    /// All the rest — host-key verification, authentication, channel
    /// model — is unchanged from the direct path.
    ///
    /// This entry point cannot dial through `EgressKind::SshJump`
    /// (that kind needs an [`crate::egress::EgressContext`]); use
    /// [`Self::connect_with_egress_ctx`] when ssh-jump is in scope.
    pub async fn connect_with_egress(
        config: &SshConfig,
        verifier: HostKeyVerifier,
        egress: Option<&crate::egress::EgressProfile>,
    ) -> Result<Self> {
        Self::connect_with_egress_ctx(config, verifier, egress, None).await
    }

    /// Like [`Self::connect_with_egress`], but accepts an optional
    /// [`crate::egress::EgressContext`] the resolver hands off to
    /// when the profile is `EgressKind::SshJump`. All other kinds
    /// ignore `ctx` — they don't need outside help to dial.
    pub async fn connect_with_egress_ctx(
        config: &SshConfig,
        verifier: HostKeyVerifier,
        egress: Option<&crate::egress::EgressProfile>,
        ctx: Option<&dyn crate::egress::EgressContext>,
    ) -> Result<Self> {
        if !config.is_valid() {
            return Err(SshError::InvalidConfig(
                "host, user, port and auth must all be set".to_string(),
            ));
        }

        let russh_config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(300)),
            keepalive_interval: Some(Duration::from_secs(30)),
            // russh defaults this to 3, so three unanswered keepalives
            // (~90s) would drop an otherwise-fine session. For a terminal
            // app that's too twitchy on a flaky link — a brief Wi-Fi blip
            // or laptop suspend shorter than the window should survive.
            // 6 × 30s ≈ 3 min of tolerance before we give up; a genuinely
            // dead connection is still caught quickly at the op layer
            // (see run_with_session_retry's is_closed eviction).
            keepalive_max: 6,
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

        // Apply the user-configured connect timeout. `0` = OS
        // default (= whatever russh's internal default does).
        let connect_result = if let Some(profile) = egress {
            // Egress path: dial through the profile, then hand the
            // resulting byte stream to russh via connect_stream.
            let dial_fut = async {
                let stream = crate::egress::resolve_tcp_with(
                    Some(profile),
                    &config.host,
                    config.port,
                    ctx,
                )
                .await
                .map_err(SshError::Connect)?;
                client::connect_stream(russh_config, stream, handler)
                    .await
                    .map_err(SshError::from)
            };
            if config.connect_timeout_secs > 0 {
                let timeout = Duration::from_secs(config.connect_timeout_secs);
                match tokio::time::timeout(timeout, dial_fut).await {
                    Ok(inner) => inner,
                    Err(_) => return Err(SshError::Timeout(timeout)),
                }
            } else {
                dial_fut.await
            }
        } else {
            // Direct path: identical behavior to the previous
            // implementation, byte-for-byte.
            let addr = config.address();
            let connect_fut = client::connect(russh_config, addr, handler);
            if config.connect_timeout_secs > 0 {
                let timeout = Duration::from_secs(config.connect_timeout_secs);
                match tokio::time::timeout(timeout, connect_fut).await {
                    Ok(inner) => inner.map_err(map_connect_error),
                    Err(_) => return Err(SshError::Timeout(timeout)),
                }
            } else {
                connect_fut.await.map_err(map_connect_error)
            }
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
                return Err(e);
            }
        };

        let mut session = Self {
            handle: Arc::new(handle),
            sudo_password: Arc::new(RwLock::new(None)),
            elevation: Arc::new(RwLock::new(crate::sudo::Elevation::Sudo)),
            elevation_armed: Arc::new(RwLock::new(false)),
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

    /// Blocking sibling of [`Self::connect_with_egress`]. Same
    /// runtime restrictions as [`Self::connect_blocking`].
    pub fn connect_with_egress_blocking(
        config: &SshConfig,
        verifier: HostKeyVerifier,
        egress: Option<&crate::egress::EgressProfile>,
    ) -> Result<Self> {
        runtime::shared().block_on(Self::connect_with_egress(config, verifier, egress))
    }

    /// Blocking sibling of [`Self::connect_with_egress_ctx`]. Same
    /// runtime restrictions as [`Self::connect_blocking`].
    pub fn connect_with_egress_ctx_blocking(
        config: &SshConfig,
        verifier: HostKeyVerifier,
        egress: Option<&crate::egress::EgressProfile>,
        ctx: Option<&dyn crate::egress::EgressContext>,
    ) -> Result<Self> {
        runtime::shared()
            .block_on(Self::connect_with_egress_ctx(config, verifier, egress, ctx))
    }

    /// Run every authentication method the config specifies, in
    /// order, until one succeeds. Records which ones we tried so
    /// the [`SshError::AuthRejected`] variant can surface that to
    /// the UI.
    async fn authenticate(&mut self, config: &SshConfig) -> Result<()> {
        let mut tried = Vec::new();

        match &config.auth {
            AuthMethod::DirectPassword { password } => {
                self.try_password_or_keyboard_interactive(
                    &config.user,
                    password,
                    "password (in-memory)",
                    &mut tried,
                )
                .await?;
            }
            AuthMethod::KeychainPassword { credential_id } => {
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
                self.try_password_or_keyboard_interactive(
                    &config.user,
                    &password,
                    &format!("password (keychain={credential_id})"),
                    &mut tried,
                )
                .await?;
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

    /// Password saved by Pier-X must behave like the system `ssh`
    /// client's password prompt. Many PAM-backed servers advertise
    /// that prompt as `keyboard-interactive` instead of the SSH
    /// `password` method, so try both with the same secret before
    /// reporting a rejection.
    async fn try_password_or_keyboard_interactive(
        &mut self,
        user: &str,
        password: &str,
        password_label: &str,
        tried: &mut Vec<String>,
    ) -> Result<()> {
        tried.push(password_label.to_string());
        match self.try_password_auth(user, password).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::debug!(
                    "password auth via {password_label} failed; trying keyboard-interactive: {e}"
                );
            }
        }

        tried.push("keyboard-interactive".to_string());
        match self
            .try_keyboard_interactive_with_password(user, password)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                log::debug!("keyboard-interactive password fallback failed: {e}");
                Err(SshError::AuthRejected {
                    tried: tried.clone(),
                })
            }
        }
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
        let mut channel = self.handle.channel_open_session().await?;
        // `want_reply = true` on both requests is load-bearing. With
        // `false` (the old behaviour) the server's accept/reject of the
        // PTY + shell was invisible: when a brand-new connection rejected
        // or closed the shell channel under the channel contention of a
        // session restore — the shell open racing the SFTP subsystem and
        // the concurrent service-detector exec channels — this method
        // still returned `Ok`, and the dead channel only surfaced when
        // the first resize/write hit it ("ssh channel task has exited"),
        // by which point the create-retry ladder had no error to act on.
        // Asking for a reply lets `confirm_shell_started` turn a failed
        // open into a real `Err` the caller can retry.
        //
        // xterm-256color matches what terminal::pty::UnixPty pins for
        // local shells, so TUIs like vim and htop render correctly.
        channel
            .request_pty(
                true,
                "xterm-256color",
                cols as u32,
                rows as u32,
                0,
                0,
                &[],
            )
            .await?;
        channel.request_shell(true).await?;
        let prelude = Self::confirm_shell_started(&mut channel).await?;
        Ok(SshChannelPty::spawn(channel, cols, rows, prelude))
    }

    /// Wait for the server to confirm the PTY + shell requests issued by
    /// [`Self::open_shell_channel`], returning any shell output that
    /// arrived during confirmation so the first prompt is not dropped.
    ///
    /// Resolves `Ok` as soon as the shell proves itself live — either
    /// both requests are acknowledged (two `CHANNEL_SUCCESS`, which the
    /// SSH spec delivers in request order) or the shell emits its first
    /// byte. Resolves `Err` on an explicit `CHANNEL_FAILURE`, an early
    /// channel close, a dropped transport, or a timeout, all of which
    /// mean the channel is unusable and the caller should retry rather
    /// than hand back a dead PTY.
    async fn confirm_shell_started(
        channel: &mut russh::Channel<russh::client::Msg>,
    ) -> Result<Vec<u8>> {
        use russh::ChannelMsg;

        let mut prelude: Vec<u8> = Vec::new();
        let mut acks = 0u8;
        let wait = async {
            loop {
                match channel.wait().await {
                    Some(ChannelMsg::Success) => {
                        acks += 1;
                        // Two acks = PTY accepted + shell started.
                        if acks >= 2 {
                            return Ok(());
                        }
                    }
                    // Any output is definitive proof the shell is live,
                    // regardless of how the server ordered its replies.
                    Some(ChannelMsg::Data { data }) => {
                        prelude.extend_from_slice(&data);
                        return Ok(());
                    }
                    Some(ChannelMsg::ExtendedData { data, ext }) => {
                        if ext == 1 {
                            prelude.extend_from_slice(&data);
                        }
                        return Ok(());
                    }
                    Some(ChannelMsg::Failure) => {
                        return Err(SshError::InvalidConfig(
                            "failed to open channel: server rejected the shell request"
                                .to_string(),
                        ));
                    }
                    Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                        return Err(SshError::InvalidConfig(
                            "failed to open channel: server closed the shell channel during open"
                                .to_string(),
                        ));
                    }
                    // ExitStatus / WindowAdjusted / etc. — keep waiting.
                    Some(_) => {}
                }
            }
        };

        match tokio::time::timeout(Duration::from_secs(15), wait).await {
            Ok(Ok(())) => Ok(prelude),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SshError::InvalidConfig(
                "failed to open channel: timed out waiting for the shell to start".to_string(),
            )),
        }
    }

    /// Sync convenience for [`Self::open_shell_channel`].
    pub fn open_shell_channel_blocking(&self, cols: u16, rows: u16) -> Result<SshChannelPty> {
        runtime::shared().block_on(self.open_shell_channel(cols, rows))
    }

    /// Open a `direct-tcpip` channel through this session and box it
    /// as an [`crate::egress::EgressStream`]. The wrapper retains a
    /// clone of this [`SshSession`] so the underlying SSH transport
    /// outlives the stream — ssh-jump callers can drop their
    /// reference to the jump session immediately after dial.
    ///
    /// Used by the egress layer to implement `EgressKind::SshJump`.
    pub async fn dial_direct_tcpip(
        &self,
        target_host: &str,
        target_port: u16,
    ) -> Result<crate::egress::EgressStream> {
        let channel = self
            .handle
            .channel_open_direct_tcpip(
                target_host.to_string(),
                target_port as u32,
                "0.0.0.0".to_string(),
                0,
            )
            .await
            .map_err(SshError::Protocol)?;
        Ok(Box::new(SessionGuardedStream {
            inner: channel.into_stream(),
            _session: self.clone(),
        }))
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

    /// Set or clear the sudo / privilege-escalation password
    /// attached to this session. Subsequent calls to
    /// [`Self::exec_with_sudo`] will wrap their command in
    /// `sudo -S -p ''` and pipe this value via stdin; subsequent
    /// calls to [`Self::exec_command`] are unaffected, so the
    /// terminal / SFTP code paths keep running as the SSH user.
    pub async fn set_sudo_password(&self, password: Option<String>) {
        let mut slot = self.sudo_password.write().await;
        *slot = password;
    }

    /// Sync wrapper for [`Self::set_sudo_password`].
    pub fn set_sudo_password_blocking(&self, password: Option<String>) {
        runtime::shared().block_on(self.set_sudo_password(password));
    }

    /// Set the privilege-escalation method paired with the sudo password
    /// for [`Self::exec_with_sudo`]. The Tauri layer calls this to make
    /// the exec-panel surface follow the terminal's effective user
    /// (`Elevation::Sudo` for root, `Elevation::SudoUser{user}` to become
    /// a specific user). Defaults to `Sudo` if never called.
    pub async fn set_elevation(&self, elevation: crate::sudo::Elevation) {
        let mut slot = self.elevation.write().await;
        *slot = elevation;
    }

    /// Sync wrapper for [`Self::set_elevation`].
    pub fn set_elevation_blocking(&self, elevation: crate::sudo::Elevation) {
        runtime::shared().block_on(self.set_elevation(elevation));
    }

    /// Mark (or clear) this session as elevated-without-a-captured-secret.
    /// See [`Self::elevation_armed`]. When `true`, [`Self::exec_with_sudo`]
    /// attempts a passwordless `sudo -n` for commands that have no secret,
    /// so a terminal `sudo -i` on a NOPASSWD host is followed.
    pub async fn set_elevation_armed(&self, armed: bool) {
        let mut slot = self.elevation_armed.write().await;
        *slot = armed;
    }

    /// Sync wrapper for [`Self::set_elevation_armed`].
    pub fn set_elevation_armed_blocking(&self, armed: bool) {
        runtime::shared().block_on(self.set_elevation_armed(armed));
    }

    /// Whether this session is currently armed for passwordless elevation.
    pub fn is_elevation_armed_blocking(&self) -> bool {
        runtime::shared().block_on(async { *self.elevation_armed.read().await })
    }

    /// Async read of the armed flag — for callers already in an async
    /// context (blocking variants would deadlock the shared runtime).
    pub async fn is_elevation_armed(&self) -> bool {
        *self.elevation_armed.read().await
    }

    /// Run a command that needs root, following the session's elevation,
    /// given a precomputed `is_root` (callers probe `id -u` once and reuse
    /// it). This is the single correct primitive for service modules
    /// (nginx / web_server / …) that used to hand-build a `sudo -n` prefix
    /// and then — buggily — run it through plain [`Self::exec_command`],
    /// which left an *empty* prefix unelevated (the command silently ran as
    /// the login user). Routing through [`Self::exec_with_sudo`] instead
    /// pipes the password via stdin (`sudo -S`, never the command line) or
    /// uses a passwordless `sudo -n` when the terminal is armed:
    ///
    /// - already root → run directly, no sudo.
    /// - captured password OR armed (terminal `sudo -i`/`su`) →
    ///   [`Self::exec_with_sudo`] (sudo -S / sudo -n, degrades cleanly).
    /// - otherwise → best-effort `sudo -n` so a NOPASSWD host the operator
    ///   hasn't elevated on still works (the legacy default).
    pub async fn exec_maybe_sudo(&self, command: &str, is_root: bool) -> Result<(i32, String)> {
        if is_root {
            return self.exec_command(command).await;
        }
        if self.has_sudo_password().await || self.is_elevation_armed().await {
            return self.exec_with_sudo(command).await;
        }
        self.exec_command(&format!("LC_ALL=C sudo -n {command}")).await
    }

    /// True when this session has a non-empty sudo password attached.
    /// Used by service modules (nginx, web_server) to decide whether
    /// they should still prepend `sudo -n ` to commands as a NOPASSWD
    /// fallback, or trust [`Self::exec_with_sudo`] to wrap with
    /// `sudo -S` and pipe the password instead.
    pub async fn has_sudo_password(&self) -> bool {
        self.sudo_password
            .read()
            .await
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// Sync convenience for [`Self::has_sudo_password`].
    pub fn has_sudo_password_blocking(&self) -> bool {
        runtime::shared().block_on(self.has_sudo_password())
    }

    /// True when this session can plausibly run a command elevated —
    /// either a secret is attached (`sudo -S` / `su`) **or** the host is
    /// armed for passwordless `sudo -n` (terminal `sudo -i` on a NOPASSWD
    /// / cached-creds host, no secret to capture). Panels gate their
    /// permission-denied → sudo fallback on this so the right side follows
    /// the terminal even when nothing was captured.
    pub fn can_elevate_blocking(&self) -> bool {
        self.has_sudo_password_blocking() || self.is_elevation_armed_blocking()
    }

    /// Snapshot the armed elevation secret. Lets the streaming-exec
    /// wrapper (in a sibling module that can't see the private slot)
    /// follow the session's elevation. `None` when nothing is armed.
    pub async fn sudo_password_snapshot(&self) -> Option<String> {
        self.sudo_password.read().await.clone()
    }

    /// Run `command` remotely. If a sudo password has been
    /// attached via [`Self::set_sudo_password`], the command is
    /// wrapped in `sudo -S -p ''` and the password is piped via
    /// stdin so the prompt never reaches the user. Otherwise this
    /// is equivalent to [`Self::exec_command`].
    ///
    /// Used by panels that act on root-only resources (Docker
    /// daemon socket, iptables, nginx reload). A first attempt
    /// without sudo can be detected via
    /// [`crate::sudo::is_permission_denied`] on the merged output;
    /// on hit, the panel prompts the user, calls
    /// `set_sudo_password`, and re-runs through this method.
    pub async fn exec_with_sudo(&self, command: &str) -> Result<(i32, String)> {
        let pw = { self.sudo_password.read().await.clone() };
        match pw {
            Some(pw) if !pw.is_empty() => {
                // Dispatch through the per-call primitive using the
                // session's stored elevation method (defaults to
                // `Sudo`, so this stays `sudo -S` as before; the Tauri
                // layer can switch it to `sudo -u <user>` to follow the
                // terminal's effective user). Audit logging lives in
                // `exec_as_effective`.
                let elevation = { self.elevation.read().await.clone() };
                self.exec_as_effective(command, &elevation, Some(&pw)).await
            }
            _ => {
                // No captured secret. If the host is *armed* (the operator
                // elevated in the terminal on a NOPASSWD / cached-creds
                // host, so there was no prompt to capture) try a
                // passwordless `sudo -n`; on a host that really needs a
                // password it fails fast (no prompt, no hang) and we
                // degrade to an unprivileged run rather than failing the
                // op. Not armed → plain exec as before.
                let armed = { *self.elevation_armed.read().await };
                if !armed {
                    return self.exec_command(command).await;
                }
                let elevation = { self.elevation.read().await.clone() };
                match crate::sudo::wrap_command_nopasswd(command, &elevation) {
                    Some(wrapped) => {
                        let res = self.exec_command(&wrapped).await?;
                        if res.0 == 0 {
                            return Ok(res);
                        }
                        // `sudo -n` refused (needs a password / not a
                        // sudoer) → run unprivileged. A non-zero exit with
                        // *other* output is the inner command failing on
                        // its own, so pass that through unchanged.
                        if crate::sudo::is_elevation_auth_failure(&res.1)
                            || crate::sudo::is_permission_denied(&res.1)
                        {
                            log::info!(
                                "[audit] passwordless sudo -n unavailable, running unprivileged"
                            );
                            self.exec_command(command).await
                        } else {
                            Ok(res)
                        }
                    }
                    None => self.exec_command(command).await,
                }
            }
        }
    }

    /// Sync convenience for [`Self::exec_with_sudo`].
    pub fn exec_with_sudo_blocking(&self, command: &str) -> Result<(i32, String)> {
        runtime::shared().block_on(self.exec_with_sudo(command))
    }

    /// Run `command` at the given privilege level, deciding elevation
    /// **per-call**. Unlike [`Self::exec_with_sudo`] this neither reads
    /// nor writes the session's sudo-password slot, so concurrent
    /// callers on a shared SSH session can run at different privilege
    /// levels without clobbering each other's elevation state — the
    /// cross-panel leak the slot mechanism is prone to.
    ///
    /// `secret` is the password for [`Elevation::Sudo`] (caller's own)
    /// and [`Elevation::Su`] (the target user's); it is ignored for
    /// [`Elevation::None`]. The secret is piped via stdin, never placed
    /// on the command line.
    pub async fn exec_as_effective(
        &self,
        command: &str,
        elevation: &crate::sudo::Elevation,
        secret: Option<&str>,
    ) -> Result<(i32, String)> {
        use crate::sudo::Elevation;
        if !matches!(elevation, Elevation::None) {
            // Audit: log the elevation method + first 80 chars of the
            // command, never the secret. Mirrors `exec_with_sudo`'s audit
            // trail so consolidating onto this primitive doesn't lose it.
            let preview: String = command.chars().take(80).collect();
            log::info!("[audit] exec_as_effective ({elevation:?}): {preview}");
        }
        match elevation {
            Elevation::None => self.exec_command(command).await,
            Elevation::Sudo => {
                let (wrapped, stdin) = crate::sudo::wrap_command(command, secret.unwrap_or(""));
                let res = self.exec_command_with_stdin(&wrapped, &stdin).await?;
                // The secret may actually be a *root* password (operator
                // `su`'d in the terminal), which `sudo` rejects. Fall back
                // to `su - root` over a PTY with the same secret so the
                // panel still follows the terminal's elevation.
                self.su_fallback_if_auth_failed(res, "root", command, secret).await
            }
            Elevation::SudoUser { target_user } => {
                let (wrapped, stdin) =
                    crate::sudo::wrap_command_sudo_u(command, target_user, secret.unwrap_or(""));
                let res = self.exec_command_with_stdin(&wrapped, &stdin).await?;
                self.su_fallback_if_auth_failed(res, target_user, command, secret)
                    .await
            }
            Elevation::Su { target_user } => {
                self.exec_su_pty(target_user, secret.unwrap_or(""), command).await
            }
        }
    }

    /// If a `sudo` attempt failed at the auth/authorization stage (wrong
    /// password / not a sudoer / needs a tty), retry the same command as
    /// `su - <target_user>` over a PTY with the same secret. Otherwise
    /// pass the sudo result through unchanged. This makes the panels
    /// follow a terminal `su root` (where the captured secret is root's
    /// password) without a per-command "method" flag.
    async fn su_fallback_if_auth_failed(
        &self,
        sudo_result: (i32, String),
        target_user: &str,
        command: &str,
        secret: Option<&str>,
    ) -> Result<(i32, String)> {
        let (code, ref out) = sudo_result;
        let preview: String = out.chars().take(160).collect();
        log::info!(
            "[audit] sudo attempt exit={code} out_len={} out_preview={preview:?}",
            out.len()
        );
        // Fall back to `su` when sudo failed to *authorize* — either an
        // explicit auth-failure string, or a non-zero exit with no output
        // (some sudo configs reject silently). A non-zero exit *with*
        // real output is the elevated command itself failing → don't
        // re-run it via su (avoids double-executing a mutation).
        let looks_auth = crate::sudo::is_elevation_auth_failure(out);
        let silent = code != 0 && out.trim().is_empty();
        if code != 0 && (looks_auth || silent) {
            log::info!(
                "[audit] sudo did not authorize (auth={looks_auth} silent={silent}), falling back to su - {target_user}"
            );
            let su_res = self.exec_su_pty(target_user, secret.unwrap_or(""), command).await?;
            log::info!(
                "[audit] su fallback exit={} out_len={}",
                su_res.0,
                su_res.1.len()
            );
            return Ok(su_res);
        }
        Ok(sudo_result)
    }

    /// Run `command` as `su - <target_user> -c` over a **PTY** channel,
    /// feeding `password` when the `Password:` prompt appears — the only
    /// way to drive `su` non-interactively (it reads from `/dev/tty`, not
    /// stdin). A sentinel `echo` brackets the real output so the password
    /// prompt + shell noise can be stripped; CRLF (from the PTY) is
    /// normalized to LF. Returns `(exit_code, clean_output)`.
    async fn exec_su_pty(
        &self,
        target_user: &str,
        password: &str,
        command: &str,
    ) -> Result<(i32, String)> {
        let user = if target_user.is_empty() {
            "root"
        } else {
            target_user
        };
        // Sentinel marks where the real command output begins, so the
        // `Password:` prompt and any login noise are dropped.
        const SENTINEL: &str = "__PIERX_SU_BEGIN__";
        let inner = format!("echo {SENTINEL}; {command}");
        let escaped = inner.replace('\'', r"'\''");
        // Force a C locale on `su` itself so its prompt is always the ASCII
        // `Password: ` (a zh_CN remote prints `密码：`, which our matcher
        // can't see — su then parks waiting for input that never comes) and
        // so failure strings like `Authentication failure` stay matchable.
        // `su -` still resets the env for the inner login shell, so the
        // command runs under the target user's normal locale.
        let full = format!("LC_ALL=C su - {user} -c '{escaped}'");

        let mut channel = self.handle.channel_open_session().await?;
        // A minimal PTY: `dumb` term keeps escape sequences out of the
        // output, small fixed size, no special modes. `want_reply=true`
        // so the PTY is confirmed allocated before we exec `su`.
        channel
            .request_pty(true, "dumb", 80, 24, 0, 0, &[])
            .await?;
        channel.exec(true, full.as_bytes()).await?;

        let mut full_out: Vec<u8> = Vec::new();
        // su re-prompts up to 3× on a wrong entry; feed on each fresh
        // prompt (matched on newly-arrived bytes, not the cumulative buffer)
        // and stop once the sentinel proves we're past auth. Without this a
        // single feed leaves the 2nd prompt unanswered and the channel hangs.
        const MAX_PW_ATTEMPTS: u8 = 3;
        let mut pw_attempts: u8 = 0;
        let mut exit_code: i32 = -1;
        // Idle timeout: if `su` blocks waiting for input we never recognised
        // (an unmatched prompt locale, a password we never fed, a hung PAM
        // module) the channel never closes and `channel.wait()` parks
        // forever. Bound each wait so a stuck prompt surfaces as an error the
        // caller can show, instead of an infinite "loading" spinner.
        const SU_IDLE: Duration = Duration::from_secs(20);
        loop {
            let msg = match tokio::time::timeout(SU_IDLE, channel.wait()).await {
                Ok(Some(msg)) => msg,
                Ok(None) => break, // channel closed — normal completion
                Err(_) => {
                    let _ = channel.close().await;
                    let preview: String =
                        String::from_utf8_lossy(&full_out).chars().take(160).collect();
                    log::warn!(
                        "[audit] su - {user} timed out after {SU_IDLE:?} idle pw_attempts={pw_attempts} raw_len={} raw_preview={preview:?}",
                        full_out.len()
                    );
                    return Err(SshError::InvalidConfig(format!(
                        "su - {user} timed out waiting for the password prompt \
                         (no response in {}s). Check that the elevation password \
                         is correct and that `su` is permitted for this account.",
                        SU_IDLE.as_secs()
                    )));
                }
            };
            match msg {
                russh::ChannelMsg::Data { data } => {
                    full_out.extend_from_slice(&data);
                    let seen = String::from_utf8_lossy(&full_out);
                    // Bail the moment su reports a bad password rather than
                    // waiting on the idle timeout — su would otherwise emit a
                    // fresh prompt that, once we exhaust our attempts, nobody
                    // answers.
                    if !seen.contains(SENTINEL)
                        && crate::sudo::is_elevation_auth_failure(&seen)
                    {
                        let _ = channel.close().await;
                        break;
                    }
                    // su's prompt is `Password: ` (no echo). Match the
                    // freshly-arrived chunk so a stale cumulative match doesn't
                    // suppress a re-prompt, and cap the attempts.
                    let chunk = String::from_utf8_lossy(&data);
                    if !seen.contains(SENTINEL)
                        && pw_attempts < MAX_PW_ATTEMPTS
                        && (chunk.contains("assword") || chunk.contains("Password"))
                    {
                        let _ = channel.data(format!("{password}\n").as_bytes()).await;
                        pw_attempts += 1;
                    }
                }
                russh::ChannelMsg::ExtendedData { data, ext: _ } => {
                    full_out.extend_from_slice(&data);
                }
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = exit_status as i32;
                }
                russh::ChannelMsg::Eof | russh::ChannelMsg::Close => {}
                _ => {}
            }
        }

        let text = String::from_utf8_lossy(&full_out).replace("\r\n", "\n");
        let sentinel_found = text.contains(SENTINEL);
        // Everything from the sentinel onward is the real output. If the
        // sentinel never appeared, auth failed (su exits before running
        // the command) — return the raw text so the caller can surface it.
        let cleaned = match text.find(SENTINEL) {
            Some(idx) => {
                let after = &text[idx + SENTINEL.len()..];
                after.strip_prefix('\n').unwrap_or(after).to_string()
            }
            None => text.clone(),
        };
        // Diagnostics (no secret): how many prompts we answered, did su
        // authenticate (sentinel reached), what came back?
        let raw_preview: String = text.chars().take(160).collect();
        log::info!(
            "[audit] su - {user} exit={exit_code} pw_attempts={pw_attempts} sentinel={sentinel_found} raw_len={} raw_preview={raw_preview:?}",
            text.len()
        );
        Ok((exit_code, cleaned))
    }

    /// Sync convenience for [`Self::exec_as_effective`].
    pub fn exec_as_effective_blocking(
        &self,
        command: &str,
        elevation: &crate::sudo::Elevation,
        secret: Option<&str>,
    ) -> Result<(i32, String)> {
        runtime::shared().block_on(self.exec_as_effective(command, elevation, secret))
    }

    /// Run `command` remotely, sending `stdin` as standard input
    /// before reading output. Returns `(exit_code, merged_stdout
    /// + stderr)` on the same contract as [`Self::exec_command`].
    ///
    /// Used by [`Self::exec_with_sudo`] to pipe the elevation
    /// password into `sudo -S` without exposing it on the command
    /// line (where it would show up in `/proc/<pid>/cmdline` and
    /// in the host's bash history if the user ever copy-pasted).
    pub async fn exec_command_with_stdin(
        &self,
        command: &str,
        stdin: &str,
    ) -> Result<(i32, String)> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;

        if !stdin.is_empty() {
            channel.data(stdin.as_bytes()).await?;
        }
        // Always EOF so the remote `sudo -S` reader doesn't block
        // waiting for more bytes.
        channel.eof().await?;

        let mut full = Vec::new();
        let mut line_buf: Vec<u8> = Vec::new();
        let mut exit_code: i32 = -1;
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => {
                    Self::drain_chunk(&data, &mut full, &mut line_buf, &mut |_| {});
                }
                russh::ChannelMsg::ExtendedData { data, ext: _ } => {
                    Self::drain_chunk(&data, &mut full, &mut line_buf, &mut |_| {});
                }
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    exit_code = exit_status as i32;
                }
                russh::ChannelMsg::Eof | russh::ChannelMsg::Close => {}
                _ => {}
            }
        }
        Ok((exit_code, String::from_utf8_lossy(&full).into_owned()))
    }

    /// Sync convenience for [`Self::exec_command_with_stdin`].
    pub fn exec_command_with_stdin_blocking(
        &self,
        command: &str,
        stdin: &str,
    ) -> Result<(i32, String)> {
        runtime::shared().block_on(self.exec_command_with_stdin(command, stdin))
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

    /// Streaming counterpart to [`Self::exec_command_with_stdin`]: run
    /// `command`, write `stdin` then EOF before reading, and stream every
    /// complete output line through `on_line`. Same `(exit_code, merged
    /// output)` contract and cancellation semantics as
    /// [`Self::exec_command_streaming`].
    ///
    /// Used by the package modules to drive `sudo -S` installs/service
    /// actions: the elevation password rides on stdin instead of being
    /// baked into the command string, where `/proc/<pid>/cmdline` would
    /// expose it to other users on the remote host.
    pub async fn exec_command_streaming_with_stdin<F>(
        &self,
        command: &str,
        stdin: &str,
        mut on_line: F,
        cancel: Option<CancellationToken>,
    ) -> Result<(i32, String)>
    where
        F: FnMut(&str),
    {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command).await?;

        if !stdin.is_empty() {
            channel.data(stdin.as_bytes()).await?;
        }
        // Always EOF so `sudo -S` (and the inner command) stop waiting for
        // more input the moment the password line is consumed.
        channel.eof().await?;

        let mut full = Vec::new();
        let mut line_buf: Vec<u8> = Vec::new();
        let mut exit_code: i32 = -1;
        let mut cancelled = false;
        loop {
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
                break;
            };
            match msg {
                russh::ChannelMsg::Data { data } => {
                    Self::drain_chunk(&data, &mut full, &mut line_buf, &mut on_line);
                }
                russh::ChannelMsg::ExtendedData { data, ext: _ } => {
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
            let _ = channel.close().await;
            exit_code = CANCELLED_EXIT_CODE;
        }
        if !line_buf.is_empty() {
            let trimmed = line_buf.strip_suffix(b"\r").unwrap_or(&line_buf);
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
        VerifyError::UserRejected { host, .. } => SshError::HostKeyRejected { host },
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
            .verify_async(&self.host, self.port, server_public_key)
            .await
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

/// AsyncRead/AsyncWrite wrapper that owns a clone of the underlying
/// [`SshSession`] alongside the channel stream. Drop ordering is
/// `inner` → `_session`, so the russh transport can finish flushing
/// before the session handle goes away.
struct SessionGuardedStream<S> {
    inner: S,
    _session: SshSession,
}

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for SessionGuardedStream<S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for SessionGuardedStream<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
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
