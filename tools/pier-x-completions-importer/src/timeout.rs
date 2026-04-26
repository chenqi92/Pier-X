//! Process timeout utility — every spawn the importer makes must
//! cap at a few seconds. Without this, commands like `vim
//! completion zsh` open the editor and hang the whole batch
//! (vim doesn't recognise the args and drops into interactive
//! mode waiting for input).
//!
//! Implementation is portable: we spawn the child, wait on a
//! background thread, and fall back to `kill` after the deadline.
//! The `wait_timeout` crate would be one line shorter but adding
//! a dep just for this isn't worth it — `try_wait` polling at
//! 50ms gives the same effective behaviour.

use std::io;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Run `cmd` and return its captured `Output` as long as it
/// finishes within `deadline`. After the deadline elapses the
/// child is sent SIGKILL and the call returns
/// `io::ErrorKind::TimedOut`. stdin is redirected from
/// `/dev/null` so `read`-blocked tools (vim, less, …) receive
/// EOF immediately on read.
pub fn run_with_timeout(mut cmd: Command, deadline: Duration) -> io::Result<Output> {
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let started = Instant::now();
    loop {
        match child.try_wait()? {
            Some(_status) => {
                return child.wait_with_output();
            }
            None => {
                if started.elapsed() >= deadline {
                    let _ = kill_tree(&mut child);
                    let _ = child.wait_with_output();
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("child exceeded {:?}", deadline),
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

/// SIGKILL the immediate child. The caller is responsible for
/// the (rare) escape where the child has spawned grandchildren —
/// `man` for example pipes through `pager`. Best-effort; we
/// don't fail the batch if kill fails.
#[cfg(unix)]
fn kill_tree(child: &mut Child) -> io::Result<()> {
    child.kill()
}

#[cfg(not(unix))]
fn kill_tree(child: &mut Child) -> io::Result<()> {
    child.kill()
}

/// Default per-command timeout. 5s is enough for slow `man` /
/// `--help` paths on cold filesystem caches; anything longer is
/// almost certainly a hung interactive process.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Commands that universally hang in `<cmd> completion zsh`
/// because they treat unrecognised args as a file to open. We
/// skip the completion-zsh source for these without spawning.
/// The man / --help paths still run.
pub const SKIP_COMPLETION_ZSH: &[&str] = &[
    "vim", "nvim", "emacs", "nano", "less", "more", "view", "ed",
];
