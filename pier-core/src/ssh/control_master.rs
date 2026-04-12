//! SSH ControlMaster — reuse the terminal's SSH connection via a
//! Unix domain socket.
//!
//! When the user types `ssh user@host` in a local terminal, the SSH
//! process can create a ControlMaster socket. Right-panel tools
//! execute commands through this socket without opening a new TCP
//! connection — they inherit the exact same auth context (including
//! su, sudo, jump hosts).
//!
//! Socket path convention: `/tmp/pier-x-ssh-{user}@{host}:{port}`

use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

/// A ControlMaster session bound to a specific host.
pub struct ControlMasterSession {
    socket_path: PathBuf,
    host: String,
    user: String,
    port: u16,
}

impl ControlMasterSession {
    /// Create a new session descriptor (does NOT connect yet).
    pub fn new(host: &str, user: &str, port: u16) -> Self {
        let socket_path = PathBuf::from(format!(
            "/tmp/pier-x-ssh-{}@{}:{}",
            user, host, port
        ));
        Self {
            socket_path,
            host: host.to_string(),
            user: user.to_string(),
            port,
        }
    }

    /// Socket file path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Wait for the socket file to appear (up to `timeout_secs`).
    /// Returns true if the socket is available.
    pub fn wait_for_socket(&self, timeout_secs: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(timeout_secs as u64);
        while Instant::now() < deadline {
            if self.is_alive() {
                return true;
            }
            thread::sleep(Duration::from_millis(500));
        }
        false
    }

    /// Spawn a background SSH ControlMaster process if the socket
    /// doesn't exist yet. This creates the socket file that other
    /// SSH processes can multiplex through.
    pub fn spawn_master(&self) -> Result<(), String> {
        if self.socket_path.exists() {
            return Ok(());
        }
        let status = Command::new("ssh")
            .args([
                "-o", &format!("ControlMaster=auto"),
                "-o", &format!("ControlPath={}", self.socket_path.display()),
                "-o", "ControlPersist=600",
                "-o", "StrictHostKeyChecking=no",
                "-o", "BatchMode=yes",
                "-o", "ConnectTimeout=10",
                "-p", &self.port.to_string(),
                "-N", "-f", // No command, fork to background
                &format!("{}@{}", self.user, self.host),
            ])
            .status()
            .map_err(|e| format!("failed to spawn ssh ControlMaster: {}", e))?;

        if !status.success() {
            return Err(format!(
                "ssh ControlMaster exited with code {}",
                status.code().unwrap_or(-1)
            ));
        }
        Ok(())
    }

    /// Check if the socket is alive using `ssh -O check`.
    pub fn is_alive(&self) -> bool {
        if !self.socket_path.exists() {
            return false;
        }
        Command::new("ssh")
            .args([
                "-o", &format!("ControlPath={}", self.socket_path.display()),
                "-O", "check",
                &format!("{}@{}", self.user, self.host),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Execute a command through the ControlMaster socket.
    /// Returns (exit_code, stdout).
    pub fn exec(&self, command: &str) -> Result<(i32, String), String> {
        let output = Command::new("ssh")
            .args([
                "-o", &format!("ControlPath={}", self.socket_path.display()),
                "-o", "StrictHostKeyChecking=no",
                "-o", "BatchMode=yes",
                "-p", &self.port.to_string(),
                &format!("{}@{}", self.user, self.host),
                command,
            ])
            .output()
            .map_err(|e| format!("ssh exec failed: {}", e))?;

        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok((code, stdout))
    }

    /// Connect: wait for socket first, then spawn master if needed.
    /// Returns true if successfully connected.
    pub fn connect(&self, timeout_secs: u32) -> bool {
        // First wait a bit for the terminal's SSH to create the socket
        let quick_wait = std::cmp::min(timeout_secs, 5);
        if self.wait_for_socket(quick_wait) {
            return true;
        }
        // Socket not found — try spawning our own ControlMaster
        if self.spawn_master().is_ok() {
            return self.wait_for_socket(5);
        }
        false
    }
}
