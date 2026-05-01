//! Long-lived VPN subprocess manager — backs both
//! [`super::EgressKind::Wireguard`] and
//! [`super::EgressKind::ExternalVpn`].
//!
//! Both kinds share the same model: spawn a system VPN client
//! (`wg-quick` / `wireguard-go` / `openvpn` / `openconnect`), let
//! the OS bring up the tun interface, and from then on every TCP
//! dial through the profile is a plain [`tokio::net::TcpStream`]
//! that rides whatever route the system installed. Per-connection
//! isolation is explicitly out of scope (see PRODUCT-SPEC §3.4).
//!
//! The subprocess is owned by [`VpnProcess`]. Drop the handle to
//! send `SIGTERM` (Unix) / kill (Windows); the corresponding VPN
//! client is responsible for tearing down its own routes on exit.
//!
//! # Privilege
//!
//! Every kind we spawn here needs root / admin to install its tun
//! interface. We do NOT auto-elevate: the user is expected to be
//! running Pier-X with a privileged shell, or to have configured
//! sudoers / runas to allow the specific binary password-free.
//! Spawning fails loudly when the binary is missing or the
//! permissions are wrong; the error bubbles up to the egress dial
//! site, which surfaces it on the next connect attempt.

use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::{EgressKind, ExternalVpnEngine};

/// One running VPN subprocess. Held inside the application's
/// lifetime cache (e.g. `AppState::vpn_processes`); when the entry
/// is removed the `Drop` impl reaps the child.
pub struct VpnProcess {
    /// Profile id this process backs.
    pub profile_id: String,
    /// Human-readable command line for diagnostics.
    pub command_line: String,
    child: Mutex<Option<Child>>,
}

impl std::fmt::Debug for VpnProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VpnProcess")
            .field("profile_id", &self.profile_id)
            .field("command_line", &self.command_line)
            .finish()
    }
}

impl Drop for VpnProcess {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                // Best effort: SIGTERM via kill, then reap. The VPN
                // client should clean its own tun on a clean exit.
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl VpnProcess {
    fn lock_child(&self) -> io::Result<MutexGuard<'_, Option<Child>>> {
        self.child
            .lock()
            .map_err(|_| io::Error::other("VpnProcess child lock poisoned"))
    }

    /// True when the underlying child is still running. Calls
    /// `try_wait`; on Windows that means the OS-level signal that
    /// the process exited has been observed.
    pub fn is_running(&self) -> bool {
        let Ok(mut guard) = self.lock_child() else { return false };
        let Some(child) = guard.as_mut() else { return false };
        match child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }
}

/// Resolve the EgressKind into the concrete spawn plan, or return
/// `None` when the kind is not a subprocess VPN. Unknown engines
/// surface as `io::Error` so the caller can present them.
fn plan_for(profile_id: &str, kind: &EgressKind) -> io::Result<Option<SpawnPlan>> {
    match kind {
        EgressKind::Wireguard {
            endpoint: _,
            address: _,
            allowed_ips: _,
            private_key: _,
            peer_public_key: _,
        } => {
            // The .conf file is expected to live under
            // ~/.config/pier-x/egress/<profile-id>.conf — written
            // by the UI when the user saves the profile. We pass
            // the path to wg-quick which knows how to bring it up.
            let conf = config_path_for(profile_id, "conf")?;
            if !conf.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "wireguard profile config missing: {} — write the WG conf there before enabling the profile",
                        conf.display()
                    ),
                ));
            }
            Ok(Some(SpawnPlan {
                program: "wg-quick".to_string(),
                args: vec!["up".to_string(), conf.display().to_string()],
                expect_tun_within: Duration::from_secs(10),
            }))
        }
        EgressKind::ExternalVpn { engine, config } => match engine {
            ExternalVpnEngine::OpenVpn => Ok(Some(SpawnPlan {
                program: "openvpn".to_string(),
                args: vec!["--config".to_string(), config.clone()],
                expect_tun_within: Duration::from_secs(20),
            })),
            ExternalVpnEngine::OpenConnect => Ok(Some(SpawnPlan {
                program: "openconnect".to_string(),
                // OpenConnect treats the bare positional arg as the
                // VPN gateway hostname (or `--config <file>`).
                args: vec![config.clone()],
                expect_tun_within: Duration::from_secs(30),
            })),
        },
        _ => Ok(None),
    }
}

/// Path under the user's data dir for per-profile config files.
/// `pier-core::paths` already exposes a similar helper for the
/// connections JSON; we keep the egress-specific files in a
/// sibling subdirectory so they're easy to back up / wipe together.
fn config_path_for(profile_id: &str, ext: &str) -> io::Result<PathBuf> {
    let base = crate::paths::data_dir()
        .ok_or_else(|| io::Error::other("no usable application data directory"))?;
    let mut p = base;
    p.push("egress");
    p.push(format!("{profile_id}.{ext}"));
    Ok(p)
}

#[derive(Debug)]
struct SpawnPlan {
    program: String,
    args: Vec<String>,
    expect_tun_within: Duration,
}

/// Start the subprocess for `kind` if the kind is a VPN subprocess
/// flavour. Returns `None` when the kind is not subprocess-backed
/// (caller should not invoke this for SOCKS / HTTP / SshJump).
///
/// Blocks until the child has been spawned and its first
/// `expect_tun_within` window has elapsed (we don't actually
/// inspect the tun list — the wait is a coarse "give the VPN time
/// to come up" before the first dial). Subsequent dials assume
/// the route is already in place.
pub fn spawn(profile_id: &str, kind: &EgressKind) -> io::Result<Option<VpnProcess>> {
    let Some(plan) = plan_for(profile_id, kind)? else {
        return Ok(None);
    };
    let mut cmd = Command::new(&plan.program);
    cmd.args(&plan.args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("spawn {} {}: {e}", plan.program, plan.args.join(" ")),
        )
    })?;
    let cmdline = format!("{} {}", plan.program, plan.args.join(" "));
    log::info!(
        "egress vpn subprocess started for profile '{profile_id}': {cmdline}"
    );
    let process = VpnProcess {
        profile_id: profile_id.to_string(),
        command_line: cmdline,
        child: Mutex::new(Some(child)),
    };

    // Coarse wait: many VPN clients exit early (auth fail, missing
    // privilege) inside the first second. Polling try_wait gives us
    // a chance to surface that as an error rather than letting the
    // first dial fail with a routing issue minutes later.
    let deadline = Instant::now() + plan.expect_tun_within.min(Duration::from_secs(2));
    while Instant::now() < deadline {
        if !process.is_running() {
            return Err(io::Error::other(format!(
                "VPN subprocess exited during startup: {}",
                process.command_line
            )));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(Some(process))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_for_socks_returns_none() {
        let kind = EgressKind::Socks5 {
            host: "127.0.0.1".into(),
            port: 1080,
            auth: None,
        };
        assert!(plan_for("p", &kind).unwrap().is_none());
    }

    #[test]
    fn plan_for_external_openvpn_returns_openvpn_command() {
        let kind = EgressKind::ExternalVpn {
            engine: ExternalVpnEngine::OpenVpn,
            config: "/tmp/example.ovpn".into(),
        };
        let plan = plan_for("p", &kind).unwrap().expect("plan");
        assert_eq!(plan.program, "openvpn");
        assert_eq!(plan.args, vec!["--config".to_string(), "/tmp/example.ovpn".into()]);
    }

    #[test]
    fn plan_for_external_openconnect_returns_openconnect_command() {
        let kind = EgressKind::ExternalVpn {
            engine: ExternalVpnEngine::OpenConnect,
            config: "vpn.corp.example.com".into(),
        };
        let plan = plan_for("p", &kind).unwrap().expect("plan");
        assert_eq!(plan.program, "openconnect");
        assert_eq!(plan.args, vec!["vpn.corp.example.com".to_string()]);
    }

    #[test]
    fn plan_for_wireguard_errors_when_conf_missing() {
        let kind = EgressKind::Wireguard {
            endpoint: "vpn.example.com:51820".into(),
            address: "10.0.0.2/32".into(),
            allowed_ips: vec![],
            private_key: super::super::EgressAuthRef {
                credential_id: "ignored".into(),
            },
            peer_public_key: "AAAA".into(),
        };
        let err = plan_for("definitely-not-a-real-profile-id", &kind).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
