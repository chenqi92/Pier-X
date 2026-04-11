//! End-to-end integration test: spawn a real child process through a
//! `UnixPty`, pipe its stdout into a `VtEmulator`, and assert the grid
//! contains what we expect.
//!
//! This is the smallest test that exercises the whole M2a surface —
//! PTY + emulator together — without touching any UI code. If this
//! passes, the Rust side of M2 is functional and M2b can start wiring
//! it into Qt with confidence.

#![cfg(unix)]

use pier_core::terminal::{Pty, UnixPty, VtEmulator};
use std::thread;
use std::time::{Duration, Instant};

/// Drain the PTY into the emulator until either `deadline` elapses
/// or the emulator's top-line text matches `needle`. Returns whether
/// we found the needle.
fn drain_until_contains(pty: &mut dyn Pty, emu: &mut VtEmulator, needle: &str, deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        match pty.read() {
            Ok(chunk) if !chunk.is_empty() => emu.process(&chunk),
            Ok(_) => thread::sleep(Duration::from_millis(10)),
            Err(_) => break,
        }
        // Scan the whole grid because /bin/echo may land on any row
        // depending on the child's TTY initialization.
        for row in 0..emu.rows {
            if emu.line_text(row).contains(needle) {
                return true;
            }
        }
    }
    false
}

#[test]
fn echo_output_lands_on_emulator_grid() {
    let mut pty = UnixPty::spawn(80, 24, "/bin/echo", &["pier-x-end-to-end"])
        .expect("forkpty failed");
    let mut emu = VtEmulator::new(80, 24);

    let found = drain_until_contains(
        &mut pty,
        &mut emu,
        "pier-x-end-to-end",
        Duration::from_secs(3),
    );
    assert!(
        found,
        "emulator grid did not contain 'pier-x-end-to-end' after draining /bin/echo output",
    );
}

#[test]
fn printf_with_ansi_colors_flows_into_cell_attributes() {
    // /usr/bin/printf is the POSIX spec'd one; /bin/printf exists on
    // macOS and Linux. We pick the macOS path first then fall back.
    let printf_path = if std::path::Path::new("/usr/bin/printf").exists() {
        "/usr/bin/printf"
    } else {
        "/bin/printf"
    };

    // printf's \033[31mRED\033[0m — the raw ESC bytes flow through
    // the pty into the emulator which should set fg = Indexed(1).
    let mut pty = UnixPty::spawn(
        80,
        24,
        printf_path,
        &["\\033[31mRED\\033[0mPLAIN"],
    )
    .expect("forkpty failed");
    let mut emu = VtEmulator::new(80, 24);

    let found = drain_until_contains(&mut pty, &mut emu, "REDPLAIN", Duration::from_secs(3));
    assert!(found, "emulator did not receive printf output");

    // Find the row containing REDPLAIN and check that the first three
    // cells of that run are red-flagged while the later cells are not.
    let row_idx = (0..emu.rows)
        .find(|&r| emu.line_text(r).contains("REDPLAIN"))
        .expect("needle found above, but row lookup failed");
    let row = &emu.cells[row_idx];
    let start_col = emu
        .line_text(row_idx)
        .find("REDPLAIN")
        .expect("needle position");

    use pier_core::terminal::Color;
    assert_eq!(row[start_col].ch, 'R');
    assert_eq!(row[start_col].fg, Color::Indexed(1));
    assert_eq!(row[start_col + 1].fg, Color::Indexed(1));
    assert_eq!(row[start_col + 2].fg, Color::Indexed(1));
    // After CSI 0 m the "P" of PLAIN should be back to default.
    assert_eq!(row[start_col + 3].ch, 'P');
    assert_eq!(row[start_col + 3].fg, Color::Default);
}
