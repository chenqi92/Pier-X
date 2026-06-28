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
//! The profile is owned by [`VpnProcess`]. Dropping the handle tears
//! the tunnel down. Two shapes are handled: long-lived daemons
//! (openvpn / openconnect) get a graceful `SIGTERM` (Unix) /
//! `TerminateProcess` (Windows) so the client removes its own
//! routes; `wg-quick` configures-and-exits, so its handle holds no
//! child and Drop runs `wg-quick down <conf>` to remove the tun.
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

/// Explicit teardown command run on `Drop` for the oneshot model
/// (`wg-quick down <conf>`). The daemon model (openvpn / openconnect)
/// has no teardown plan — its cleanup is a `SIGTERM` to the child.
#[derive(Debug, Clone)]
struct TeardownPlan {
    program: String,
    args: Vec<String>,
}

/// One running VPN profile. Held inside the application's lifetime
/// cache (e.g. `AppState::vpn_processes`); when the entry is removed
/// the `Drop` impl tears the tunnel down.
///
/// Two lifecycle shapes share this type:
/// - **daemon** (openvpn / openconnect): a long-lived `child`;
///   `Drop` sends `SIGTERM` so the client removes its own tun/routes.
/// - **oneshot** (wg-quick): `wg-quick up` configures the interface
///   and exits 0, so there is no child to hold; `Drop` runs the
///   `teardown` command (`wg-quick down`) to remove it.
pub struct VpnProcess {
    /// Profile id this process backs.
    pub profile_id: String,
    /// Human-readable command line for diagnostics.
    pub command_line: String,
    /// Long-lived child (daemon model). `None` for the oneshot model.
    child: Mutex<Option<Child>>,
    /// Teardown command (oneshot model). `None` for the daemon model.
    teardown: Option<TeardownPlan>,
    /// For the oneshot model: the configure step succeeded and the
    /// tunnel is considered up until `Drop` tears it down.
    oneshot_active: bool,
}

impl std::fmt::Debug for VpnProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VpnProcess")
            .field("profile_id", &self.profile_id)
            .field("command_line", &self.command_line)
            .finish()
    }
}

/// Reap a daemon child gracefully: `SIGTERM` first so the VPN client
/// runs its own route/tun cleanup, escalating to `SIGKILL` only if it
/// ignores the request within the grace window. On Windows there is
/// no graceful signal, so `kill()` (TerminateProcess) is all we have.
fn terminate_gracefully(child: &mut Child) {
    #[cfg(unix)]
    {
        // SAFETY: `child.id()` is a live PID we own; `libc::kill`
        // with SIGTERM is the standard graceful-stop request.
        let pid = child.id() as libc::pid_t;
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                _ => break,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for VpnProcess {
    fn drop(&mut self) {
        // Daemon model: SIGTERM the child so it tears down its tun.
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                terminate_gracefully(&mut child);
            }
        }
        // Oneshot model: the `up` command already exited, so run the
        // matching `down` command to remove the interface and routes.
        // Without this the wg-quick tun would leak on the host — the
        // exact "don't touch the host's routing" stance §3.4 promises.
        if let Some(td) = &self.teardown {
            log::info!(
                "egress vpn teardown for profile '{}': {} {}",
                self.profile_id,
                td.program,
                td.args.join(" ")
            );
            let _ = Command::new(&td.program)
                .args(&td.args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

impl VpnProcess {
    fn lock_child(&self) -> io::Result<MutexGuard<'_, Option<Child>>> {
        self.child
            .lock()
            .map_err(|_| io::Error::other("VpnProcess child lock poisoned"))
    }

    /// True when the tunnel is up. For the daemon model this means the
    /// child is still running; for the oneshot model it means the
    /// configure step succeeded (and `Drop` hasn't torn it down yet).
    pub fn is_running(&self) -> bool {
        if self.oneshot_active {
            return true;
        }
        let Ok(mut guard) = self.lock_child() else {
            return false;
        };
        let Some(child) = guard.as_mut() else {
            return false;
        };
        matches!(child.try_wait(), Ok(None))
    }
}

/// Resolve the EgressKind into the concrete spawn plan, or return
/// `None` when the kind is not a subprocess VPN. Unknown engines
/// surface as `io::Error` so the caller can present them.
/// Reject a value that would be parsed as a command-line flag.
/// Profile fields (conf paths, gateway hostnames) come from the
/// persisted store / import, so a value starting with `-` could
/// inject extra flags into the privileged VPN binary (e.g.
/// openconnect's bare positional arg, or any `--script=` style
/// option). A real path / hostname never starts with `-`.
fn reject_flaglike(value: &str, field: &str) -> io::Result<()> {
    if value.trim_start().starts_with('-') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{field} must not start with '-' (it would be parsed as a flag by the VPN binary): {value}"
            ),
        ));
    }
    Ok(())
}

fn plan_for(profile_id: &str, kind: &EgressKind) -> io::Result<Option<SpawnPlan>> {
    match kind {
        EgressKind::Wireguard { conf_path } => {
            // Honour an explicit user path first; fall back to the
            // app-managed slot only when the field is empty.
            let conf: PathBuf = if conf_path.trim().is_empty() {
                config_path_for(profile_id, "conf")?
            } else {
                reject_flaglike(conf_path, "wireguard conf_path")?;
                PathBuf::from(conf_path.trim())
            };
            if !conf.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "wireguard profile config missing: {} — point conf_path at an existing wg-quick .conf before enabling the profile",
                        conf.display()
                    ),
                ));
            }
            let conf_str = conf.display().to_string();
            Ok(Some(wireguard_plan(&conf, conf_str)?))
        }
        EgressKind::ExternalVpn {
            engine,
            config,
            protocol,
        } => {
            reject_flaglike(config, "external vpn config")?;
            match engine {
                ExternalVpnEngine::OpenVpn => Ok(Some(SpawnPlan {
                    program: "openvpn".to_string(),
                    args: vec!["--config".to_string(), config.clone()],
                    expect_tun_within: Duration::from_secs(20),
                    lifecycle: Lifecycle::Daemon,
                })),
                ExternalVpnEngine::OpenConnect => {
                    // `--protocol` selects the WebVPN dialect
                    // (anyconnect / nc / gp / pulse / f5 / fortinet /
                    // array). Omitted → the binary's AnyConnect
                    // default. Values come from a closed enum, so no
                    // injection surface here.
                    let mut args = Vec::new();
                    if let Some(p) = protocol {
                        args.push(format!("--protocol={}", p.flag_value()));
                    }
                    // OpenConnect treats the bare positional arg as the
                    // VPN gateway hostname (or `--config <file>`).
                    args.push(config.clone());
                    Ok(Some(SpawnPlan {
                        program: "openconnect".to_string(),
                        args,
                        expect_tun_within: Duration::from_secs(30),
                        lifecycle: Lifecycle::Daemon,
                    }))
                }
            }
        }
        _ => Ok(None),
    }
}

/// WireGuard spawn plan for the current platform.
///
/// Unix: `wg-quick up <conf>` (configure-and-exit) torn down by
/// `wg-quick down <conf>`.
///
/// Windows has no wg-quick — the official client installs the tunnel
/// as a Windows service instead: `wireguard.exe /installtunnelservice
/// <conf>` (service name derives from the conf file stem), torn down
/// by `wireguard.exe /uninstalltunnelservice <stem>`. Both verbs need
/// an elevated process, same privilege stance as the Unix path.
#[cfg(not(windows))]
fn wireguard_plan(_conf: &std::path::Path, conf_str: String) -> io::Result<SpawnPlan> {
    Ok(SpawnPlan {
        program: "wg-quick".to_string(),
        args: vec!["up".to_string(), conf_str.clone()],
        expect_tun_within: Duration::from_secs(10),
        // `wg-quick up` configures the interface then exits 0 —
        // it is NOT a long-lived daemon. Hold no child; tear
        // down with `wg-quick down <conf>` on Drop.
        lifecycle: Lifecycle::Oneshot {
            teardown: TeardownPlan {
                program: "wg-quick".to_string(),
                args: vec!["down".to_string(), conf_str],
            },
        },
    })
}

#[cfg(windows)]
fn wireguard_plan(conf: &std::path::Path, conf_str: String) -> io::Result<SpawnPlan> {
    let tunnel = conf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    if tunnel.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "wireguard conf filename is not a valid tunnel name: {}",
                conf.display()
            ),
        ));
    }
    let program = windows_wireguard_binary();
    Ok(SpawnPlan {
        program: program.clone(),
        args: vec!["/installtunnelservice".to_string(), conf_str],
        expect_tun_within: Duration::from_secs(10),
        // `/installtunnelservice` registers + starts the tunnel
        // service then exits — same configure-and-exit shape as
        // wg-quick, with the uninstall verb as teardown.
        lifecycle: Lifecycle::Oneshot {
            teardown: TeardownPlan {
                program,
                args: vec!["/uninstalltunnelservice".to_string(), tunnel],
            },
        },
    })
}

/// Locate `wireguard.exe`. The official installer puts it under
/// `%ProgramFiles%\WireGuard` but does not always extend PATH, so
/// probe the well-known location first and fall back to PATH lookup.
#[cfg(windows)]
fn windows_wireguard_binary() -> String {
    let program_files =
        std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into());
    let well_known = PathBuf::from(program_files)
        .join("WireGuard")
        .join("wireguard.exe");
    if well_known.exists() {
        well_known.display().to_string()
    } else {
        "wireguard.exe".to_string()
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

/// How the spawned process behaves over its lifetime.
#[derive(Debug)]
enum Lifecycle {
    /// Long-lived client (openvpn / openconnect). Hold the child;
    /// `SIGTERM` it on teardown.
    Daemon,
    /// Configure-and-exit command (wg-quick). The `up` runs to
    /// completion; tear down with a separate command on Drop.
    Oneshot { teardown: TeardownPlan },
}

#[derive(Debug)]
struct SpawnPlan {
    program: String,
    args: Vec<String>,
    expect_tun_within: Duration,
    lifecycle: Lifecycle,
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
    let cmdline = format!("{} {}", plan.program, plan.args.join(" "));

    match plan.lifecycle {
        Lifecycle::Oneshot { teardown } => {
            // wg-quick up: run to completion. A non-zero exit (bad
            // conf, missing privilege, kernel module absent) is a
            // hard failure surfaced with the client's stderr; exit 0
            // means the interface + routes are installed.
            let output = Command::new(&plan.program)
                .args(&plan.args)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| io::Error::new(e.kind(), format!("spawn {cmdline}: {e}")))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(io::Error::other(format!(
                    "{cmdline} failed ({}): {}",
                    output.status,
                    stderr.trim()
                )));
            }
            log::info!("egress vpn configured for profile '{profile_id}': {cmdline}");
            Ok(Some(VpnProcess {
                profile_id: profile_id.to_string(),
                command_line: cmdline,
                child: Mutex::new(None),
                teardown: Some(teardown),
                oneshot_active: true,
            }))
        }
        Lifecycle::Daemon => {
            let mut cmd = Command::new(&plan.program);
            cmd.args(&plan.args);
            cmd.stdin(Stdio::null());
            cmd.stdout(Stdio::null());
            cmd.stderr(Stdio::piped());
            let child = cmd
                .spawn()
                .map_err(|e| io::Error::new(e.kind(), format!("spawn {cmdline}: {e}")))?;
            log::info!("egress vpn subprocess started for profile '{profile_id}': {cmdline}");
            let process = VpnProcess {
                profile_id: profile_id.to_string(),
                command_line: cmdline,
                child: Mutex::new(Some(child)),
                teardown: None,
                oneshot_active: false,
            };

            // Coarse wait: many VPN clients exit early (auth fail,
            // missing privilege) inside the first second. Polling
            // try_wait surfaces that as an error rather than letting
            // the first dial fail with a routing issue minutes later.
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::egress::OpenConnectProtocol;

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
            protocol: None,
        };
        let plan = plan_for("p", &kind).unwrap().expect("plan");
        assert_eq!(plan.program, "openvpn");
        assert_eq!(
            plan.args,
            vec!["--config".to_string(), "/tmp/example.ovpn".into()]
        );
    }

    #[test]
    fn plan_for_external_openconnect_returns_openconnect_command() {
        let kind = EgressKind::ExternalVpn {
            engine: ExternalVpnEngine::OpenConnect,
            config: "vpn.corp.example.com".into(),
            protocol: None,
        };
        let plan = plan_for("p", &kind).unwrap().expect("plan");
        assert_eq!(plan.program, "openconnect");
        assert_eq!(plan.args, vec!["vpn.corp.example.com".to_string()]);
    }

    #[test]
    fn plan_for_openconnect_protocol_maps_to_flag() {
        let kind = EgressKind::ExternalVpn {
            engine: ExternalVpnEngine::OpenConnect,
            config: "vpn.corp.example.com".into(),
            protocol: Some(OpenConnectProtocol::Gp),
        };
        let plan = plan_for("p", &kind).unwrap().expect("plan");
        assert_eq!(
            plan.args,
            vec![
                "--protocol=gp".to_string(),
                "vpn.corp.example.com".to_string()
            ]
        );
    }

    #[test]
    fn plan_for_openvpn_ignores_protocol() {
        let kind = EgressKind::ExternalVpn {
            engine: ExternalVpnEngine::OpenVpn,
            config: "/tmp/example.ovpn".into(),
            protocol: Some(OpenConnectProtocol::Pulse),
        };
        let plan = plan_for("p", &kind).unwrap().expect("plan");
        assert_eq!(
            plan.args,
            vec!["--config".to_string(), "/tmp/example.ovpn".into()]
        );
    }

    #[test]
    fn plan_for_rejects_flaglike_openconnect_config() {
        let kind = EgressKind::ExternalVpn {
            engine: ExternalVpnEngine::OpenConnect,
            config: "--script=/tmp/evil".into(),
            protocol: None,
        };
        let err = plan_for("p", &kind).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn wireguard_plan_matches_platform_tooling() {
        let conf = std::env::temp_dir().join("pier-x-test-wg0.conf");
        std::fs::write(&conf, "[Interface]\n").expect("write temp conf");
        let kind = EgressKind::Wireguard {
            conf_path: conf.display().to_string(),
        };
        let plan = plan_for("p", &kind).unwrap().expect("plan");
        let _ = std::fs::remove_file(&conf);
        #[cfg(windows)]
        {
            assert!(plan.program.ends_with("wireguard.exe"));
            assert_eq!(plan.args[0], "/installtunnelservice");
            match plan.lifecycle {
                Lifecycle::Oneshot { teardown } => {
                    assert_eq!(
                        teardown.args,
                        vec![
                            "/uninstalltunnelservice".to_string(),
                            "pier-x-test-wg0".to_string()
                        ]
                    );
                }
                Lifecycle::Daemon => panic!("wireguard plan must be oneshot"),
            }
        }
        #[cfg(not(windows))]
        {
            assert_eq!(plan.program, "wg-quick");
            assert_eq!(plan.args[0], "up");
            match plan.lifecycle {
                Lifecycle::Oneshot { teardown } => assert_eq!(teardown.args[0], "down"),
                Lifecycle::Daemon => panic!("wireguard plan must be oneshot"),
            }
        }
    }

    #[test]
    fn plan_for_rejects_flaglike_wireguard_conf_path() {
        let kind = EgressKind::Wireguard {
            conf_path: "-x/evil.conf".into(),
        };
        let err = plan_for("p", &kind).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn plan_for_wireguard_errors_when_conf_missing() {
        let kind = EgressKind::Wireguard {
            conf_path: String::new(),
        };
        let err = plan_for("definitely-not-a-real-profile-id", &kind).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn plan_for_wireguard_uses_explicit_conf_path() {
        let kind = EgressKind::Wireguard {
            conf_path: "/definitely/not/here/wg.conf".into(),
        };
        let err = plan_for("p", &kind).unwrap_err();
        // The error message must reference the explicit path,
        // proving we honoured it (and not the default slot).
        assert!(err.to_string().contains("/definitely/not/here/wg.conf"));
    }
}
