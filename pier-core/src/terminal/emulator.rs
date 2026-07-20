//! VT100 / ANSI terminal emulator.
//!
//! Thin wrapper over the `vte` crate's state machine. We own the grid
//! (rows × cols of [`Cell`]), a cursor position, the current SGR
//! attributes (foreground / background color, bold, underline,
//! reverse), and a bounded scrollback buffer. Bytes produced by a
//! [`super::Pty`] are fed in via [`VtEmulator::process`]; the shell
//! reads cells out of [`VtEmulator::cells`] at render time.
//!
//! ## Scope today
//!
//! We handle the subset of the VT100 / ANSI protocol that real shells
//! (bash, zsh, fish) and interactive TUIs (vim, htop, less) hit most
//! often in practice:
//!
//! * printable characters with current SGR attrs
//! * `\r`, `\n`, `\t`, `\x08` (BS)
//! * CSI cursor movement `A B C D H f`, absolute `G`/`` ` `` (CHA) and
//!   `d` (VPA)
//! * CSI erase `J` and `K` (0/1/2 variants), `X` (ECH)
//! * CSI line/char editing `L` (IL), `M` (DL), `@` (ICH), `P` (DCH)
//! * CSI scroll region `r` (DECSTBM) with `S`/`T` (SU/SD) and the
//!   `ESC D`/`ESC M`/`ESC E` index sequences honouring it
//! * CSI `m` — SGR, enough of it to set fg/bg/bold/underline/reverse
//! * `ESC 7`/`ESC 8` (DECSC/DECRC) and `CSI s`/`u` cursor save-restore
//! * DECSET/DECRST `?7` (autowrap), `?47/1047/1049` (alt screen),
//!   `?2004` (bracketed paste)
//!
//! This is the set ncurses TUIs (nano, less, htop) actually emit to
//! redraw incrementally — an earlier build handled only cursor moves +
//! erase + SGR, so ncurses' column/row-absolute moves and line-insert
//! optimisations landed at the wrong cell and painted overlapping text.
//!
//! Scrolling past the bottom row shifts the top line into the
//! [`VtEmulator::scrollback`] ring (capped at `scrollback_limit`).
//!
//! Sequences we don't yet handle are silently swallowed rather than
//! printed garbage — the `vte` parser routes them to the appropriate
//! `Perform` hook which we simply leave empty. That's deliberately
//! permissive for M2a: the smoke test is "can we get a running shell
//! with a readable prompt on the screen", not "are we a pixel-perfect
//! xterm". The remaining sequences land incrementally in M2b and
//! later milestones as users hit them.

use std::collections::VecDeque;
use vte::{Parser, Perform};

/// A single cell in the terminal grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    /// Printable character. Cleared cells hold a single space.
    pub ch: char,
    /// Foreground color at the time this cell was written.
    pub fg: Color,
    /// Background color at the time this cell was written.
    pub bg: Color,
    /// Bold attribute (SGR 1).
    pub bold: bool,
    /// Underline attribute (SGR 4).
    pub underline: bool,
    /// Reverse-video attribute (SGR 7). Most UIs render this by
    /// swapping fg/bg at paint time.
    pub reverse: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            bold: false,
            underline: false,
            reverse: false,
        }
    }
}

/// Terminal color. The parser distinguishes three variants so the
/// shell can implement palette lookup the way it prefers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Whatever the theme considers "default fg" / "default bg".
    Default,
    /// Indexed into the 256-color ANSI palette (0–15 basic, 16–231
    /// cube, 232–255 grayscale).
    Indexed(u8),
    /// True color (SGR `38;2;r;g;b` / `48;2;r;g;b`).
    Rgb(u8, u8, u8),
}

/// A line evicted from the top of the grid into scrollback.
///
/// We store the full cell line (not just text) so the UI can still
/// render colored scrollback. It's not cheap — a terminal running
/// `cat` on a huge log will fill `scrollback_limit` lines with ~120
/// cells each — but it's O(rows × cols × limit) at worst and bounded.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollbackLine {
    /// The cells of this physical line.
    pub cells: Vec<Cell>,
    /// True when this line soft-wrapped into the one below it (the text
    /// reached the right margin and auto-wrapped) rather than ending at
    /// a hard newline. [`VtEmulator::resize`]'s reflow rejoins runs of
    /// soft-wrapped lines into a single logical line before re-wrapping
    /// them at the new width, so a shrink → grow round-trip restores the
    /// original layout instead of leaving lines stuck at the old width.
    pub wrapped: bool,
}

impl ScrollbackLine {
    fn new(cells: Vec<Cell>, wrapped: bool) -> Self {
        Self { cells, wrapped }
    }
}

/// VT100 state machine + grid + scrollback.
///
/// Construct with [`VtEmulator::new`], feed bytes via
/// [`VtEmulator::process`], read cells via [`VtEmulator::cells`] and
/// scrollback via [`VtEmulator::scrollback`].
pub struct VtEmulator {
    parser: Parser,

    /// Grid width, in cells.
    pub cols: usize,
    /// Grid height, in cells.
    pub rows: usize,

    /// Current cursor column, 0-based. Always within `0..cols`.
    pub cursor_x: usize,
    /// Current cursor row, 0-based. Always within `0..rows`.
    pub cursor_y: usize,

    /// The visible grid. `cells[row][col]`. `cells.len() == rows`.
    pub cells: Vec<Vec<Cell>>,

    /// Per-row soft-wrap flags, parallel to `cells` (`wrapped.len() ==
    /// rows`). `wrapped[y] == true` means row `y` auto-wrapped at the
    /// right margin into row `y + 1`, i.e. the two rows are one logical
    /// line. Set on auto-wrap in `print`, cleared by a hard newline or
    /// an erase that touches the row's right edge, and carried alongside
    /// each row through `scroll_up` / `resize`. Drives reflow on a
    /// column resize.
    pub wrapped: Vec<bool>,

    /// Bounded FIFO of lines that scrolled off the top.
    pub scrollback: VecDeque<ScrollbackLine>,

    /// Maximum number of scrollback lines to retain. Default 10_000.
    pub scrollback_limit: usize,

    /// Current pen style that the next printed character will take.
    /// CSI `m` mutates this.
    pen: Cell,

    /// Set to true when a BEL character (0x07) is received.
    /// The shell reads and resets this flag per snapshot.
    pub bell_pending: bool,

    /// Window title set via OSC 0/1/2.
    pub window_title: String,

    /// Clipboard content set via OSC 52. The shell decides
    /// whether to honor clipboard writes from the terminal.
    pub osc52_clipboard: String,

    /// Last-known current working directory reported by the
    /// shell via OSC 7 (`\x1b]7;file://host/path\x1b\\`). Empty
    /// until the remote shell's prompt hook fires — typical
    /// bash / zsh configurations on macOS and many Linux
    /// distros emit OSC 7 on every prompt redraw. Consumers
    /// should treat an empty string as "unknown, try a
    /// fallback" rather than "root".
    pub cwd: String,

    /// Last-known shell user reported by Pier-X's prompt hook.
    /// Empty when the hook has not fired or the shell does not
    /// support it. Consumers should use this as display context,
    /// not as SSH authentication material.
    pub current_user: String,

    /// SSH command detected in terminal output. Set when the user
    /// presses Enter on a line containing `ssh [user@]host`.
    /// The UI reads these and clears `ssh_command_detected`.
    pub ssh_command_detected: bool,
    /// Host extracted from the most recent detected `ssh` command.
    pub ssh_detected_host: String,
    /// User extracted from the most recent detected `ssh` command.
    pub ssh_detected_user: String,
    /// Port extracted from the most recent detected `ssh` command.
    pub ssh_detected_port: u16,

    /// Set when `exit` or `logout` is detected — signals that
    /// the user left the current SSH session.
    pub ssh_exit_detected: bool,

    /// `(row, col)` of the most recent OSC 133;B (prompt-end /
    /// "user input starts here") sequence emitted by a smart-mode
    /// shell. The smart-mode init script (`smart.rs`) wraps the
    /// user's PS1 with `\e]133;A\a` and `\e]133;B\a`; the UI uses
    /// the position of B to overlay autosuggest / syntax-highlight
    /// on top of the still-being-typed line. `None` until the first
    /// wrapped prompt is seen.
    ///
    /// The position is in grid coordinates and stays valid through
    /// scrolling: every `scroll_up` shifts it up by one row and
    /// invalidates it once it falls off the top.
    pub last_prompt_end: Option<(usize, usize)>,

    /// `true` between OSC 133;B (user starts typing) and the next
    /// `133;C` (user pressed Enter) or `133;A` (a fresh prompt was
    /// drawn without an intervening command). The smart-mode UI
    /// activates the input mirror only when this is `true`.
    pub awaiting_input: bool,

    /// `true` while the application has switched to the alternate
    /// screen buffer (DECSET 1049 / 1047 / 47). vim, htop, less,
    /// tmux all flip this on; the smart-mode UI must immediately
    /// step out of the way (no overlay, no popover) until the app
    /// switches back. We don't actually maintain a separate primary
    /// buffer — we just track the flag so the UI knows to disable
    /// itself.
    pub alt_screen: bool,

    /// `true` while the shell has bracketed-paste mode *enabled*
    /// (DECSET 2004). bash/zsh leave this on for the whole life of
    /// an interactive prompt, so this flag does NOT mean "paste in
    /// flight" — it just means readline is willing to receive
    /// `\e[200~`/`\e[201~` markers. To pause the lexer during a
    /// real multi-kB paste we'd need to track those markers
    /// separately; not implemented yet.
    pub bracketed_paste: bool,

    /// Top row of the DECSTBM scroll region (0-based, inclusive).
    /// Defaults to `0`. `line_feed` at `scroll_bottom` scrolls the
    /// rows in `scroll_top..=scroll_bottom` instead of the whole
    /// screen; `ESC M` (RI) at `scroll_top` scrolls them the other
    /// way. ncurses TUIs (nano, less, vim on the primary screen) set
    /// a region so their edit area scrolls without disturbing the
    /// status/help bars. Reset to the full screen on resize.
    pub scroll_top: usize,
    /// Bottom row of the DECSTBM scroll region (0-based, inclusive).
    /// Defaults to `rows - 1`.
    pub scroll_bottom: usize,

    /// DECAWM autowrap mode (DECSET/DECRST 7). Default on. When off,
    /// a printable character at the right margin overwrites the last
    /// cell in place instead of wrapping to the next row — TUIs turn
    /// this off while painting a full-width bottom line so the write
    /// does not scroll the screen out from under them.
    autowrap: bool,

    /// Saved cursor position + pen for DECSC/DECRC (`ESC 7` / `ESC 8`)
    /// and the ANSI.SYS `CSI s` / `CSI u`. `None` until the first save.
    saved_cursor: Option<(usize, usize, Cell)>,

    /// The last graphic character printed, for REP (`CSI Ps b`), which
    /// ncurses emits to draw runs of a repeated glyph (padding spaces,
    /// `─` rules). Defaults to a space.
    last_char: char,
}

impl VtEmulator {
    /// Construct a fresh emulator with the given grid size and a
    /// default 10k-line scrollback.
    pub fn new(cols: usize, rows: usize) -> Self {
        assert!(cols > 0 && rows > 0, "terminal grid must be at least 1x1");
        Self {
            parser: Parser::new(),
            cols,
            rows,
            cursor_x: 0,
            cursor_y: 0,
            cells: vec![vec![Cell::default(); cols]; rows],
            wrapped: vec![false; rows],
            scrollback: VecDeque::new(),
            scrollback_limit: 10_000,
            pen: Cell::default(),
            bell_pending: false,
            window_title: String::new(),
            osc52_clipboard: String::new(),
            cwd: String::new(),
            current_user: String::new(),
            ssh_command_detected: false,
            ssh_detected_host: String::new(),
            ssh_detected_user: String::new(),
            ssh_detected_port: 22,
            ssh_exit_detected: false,
            last_prompt_end: None,
            awaiting_input: false,
            alt_screen: false,
            bracketed_paste: false,
            scroll_top: 0,
            scroll_bottom: rows - 1,
            autowrap: true,
            saved_cursor: None,
            last_char: ' ',
        }
    }

    /// Feed raw bytes from a [`super::Pty`] into the parser.
    pub fn process(&mut self, bytes: &[u8]) {
        // Borrow-splitting gymnastics: the performer needs mutable
        // access to everything except `parser`, and `parser.advance`
        // needs `&mut self.parser`. We take the parser out, run it,
        // then put it back. `std::mem::take` + default is cheap for
        // `vte::Parser` (it's a handful of bytes of state).
        let mut parser = std::mem::take(&mut self.parser);
        let mut performer = Performer {
            cols: self.cols,
            rows: self.rows,
            cursor_x: &mut self.cursor_x,
            cursor_y: &mut self.cursor_y,
            cells: &mut self.cells,
            wrapped: &mut self.wrapped,
            scrollback: &mut self.scrollback,
            scrollback_limit: self.scrollback_limit,
            pen: &mut self.pen,
            bell_pending: &mut self.bell_pending,
            window_title: &mut self.window_title,
            osc52_clipboard: &mut self.osc52_clipboard,
            cwd: &mut self.cwd,
            current_user: &mut self.current_user,
            ssh_command_detected: &mut self.ssh_command_detected,
            ssh_detected_host: &mut self.ssh_detected_host,
            ssh_detected_user: &mut self.ssh_detected_user,
            ssh_detected_port: &mut self.ssh_detected_port,
            last_prompt_end: &mut self.last_prompt_end,
            awaiting_input: &mut self.awaiting_input,
            alt_screen: &mut self.alt_screen,
            bracketed_paste: &mut self.bracketed_paste,
            scroll_top: &mut self.scroll_top,
            scroll_bottom: &mut self.scroll_bottom,
            autowrap: &mut self.autowrap,
            saved_cursor: &mut self.saved_cursor,
            last_char: &mut self.last_char,
        };
        // Remember cursor row before processing to detect line changes
        // after the parser advances.
        let prev_y = *performer.cursor_y;
        parser.advance(&mut performer, bytes);
        self.parser = parser;

        // If cursor moved to a new line (user pressed Enter), check
        // the previous line for an SSH command.
        if self.cursor_y != prev_y || bytes.contains(&b'\n') || bytes.contains(&b'\r') {
            // Check the line the cursor was on before the LF
            let check_row = if prev_y < self.rows { prev_y } else { 0 };
            let line = self.line_text(check_row);
            if let Some((host, user, port)) = parse_ssh_command(&line) {
                self.ssh_detected_host = host;
                self.ssh_detected_user = user;
                self.ssh_detected_port = port;
                self.ssh_command_detected = true;
            }
        }
    }

    /// Resize the grid, preserving on-screen content across a resize.
    ///
    /// The naive approach — `Vec::resize`, which trims rows off the
    /// *bottom* on shrink — silently discarded the most recent output and
    /// the live prompt, so dragging the window smaller and back larger
    /// left the terminal truncated. On the primary screen instead:
    ///
    ///   * a **column change** triggers a full reflow: the whole buffer
    ///     (scrollback + grid) is rejoined into logical lines along the
    ///     soft-wrap flags, re-wrapped at the new width, and re-split into
    ///     scrollback + grid — so a line that wrapped at the old width
    ///     re-wraps cleanly and a shrink → grow round-trip restores the
    ///     original layout instead of leaving lines stuck at the old
    ///     width (or column-truncated);
    ///   * a **row-only change** keeps a cheaper path: shrink drops blank
    ///     rows below the cursor first (so a post-`clear` prompt near the
    ///     top is kept) then spills the top rows into scrollback; grow
    ///     pulls them back. The cursor + prompt marker move with content.
    ///
    /// The alternate screen has no scrollback and the TUI redraws itself
    /// on SIGWINCH, so it keeps a simple trim/pad.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        if cols == self.cols && rows == self.rows {
            return;
        }

        if self.alt_screen {
            // Alternate screen (no scrollback; the TUI repaints itself).
            self.cells.resize(rows, vec![Cell::default(); cols]);
            self.wrapped.resize(rows, false);
        } else {
            if cols != self.cols {
                // Re-wrap only the off-screen history to the new width...
                self.rewrap_scrollback(cols);
                // ...and leave the live grid where it is, clamping/padding each
                // visible row to the new width rather than re-wrapping it. This
                // keeps the shell's prompt on the same row so its SIGWINCH
                // repaint overwrites it in place instead of leaving a duplicate
                // one row away (xterm's primary-screen behaviour). The trade-off
                // is that an on-screen line wider than the new width is clipped
                // until the app redraws it.
                for row in self.cells.iter_mut() {
                    row.resize(cols, Cell::default());
                }
                self.cols = cols;
            }
            if rows != self.rows {
                self.resize_rows(rows);
            }
        }

        self.cols = cols;
        self.rows = rows;

        // Normalize every row to the new column count: pads rows pulled in from
        // scrollback by a row-count change, and is a cheap no-op otherwise.
        for row in self.cells.iter_mut() {
            row.resize(cols, Cell::default());
        }
        debug_assert_eq!(self.cells.len(), self.rows);
        debug_assert_eq!(self.wrapped.len(), self.rows);

        if self.cursor_x >= cols {
            self.cursor_x = cols - 1;
        }
        if self.cursor_y >= rows {
            self.cursor_y = rows - 1;
        }
        // A resize invalidates any app-set scroll region; reset it to the full
        // screen. Apps re-issue DECSTBM on their SIGWINCH redraw. Without this
        // a shrink could leave `scroll_bottom` pointing past the last row.
        self.scroll_top = 0;
        self.scroll_bottom = self.rows - 1;
        // The OSC 133;B mark refers to a cell address that may no
        // longer be inside the new bounds. Drop it rather than risk
        // a stale pointer into the grid; the next prompt redraw will
        // emit a fresh marker.
        if let Some((r, c)) = self.last_prompt_end {
            if r >= rows || c >= cols {
                self.last_prompt_end = None;
                self.awaiting_input = false;
            }
        }
    }

    /// Row-only resize (column count unchanged) on the primary screen.
    /// Shrink spills top rows into scrollback after dropping blank rows
    /// below the cursor; grow pulls them back. Soft-wrap flags travel with
    /// each row so a later column reflow still sees correct boundaries.
    fn resize_rows(&mut self, rows: usize) {
        let old_rows = self.rows;
        let cols = self.cols;
        let row_is_blank = |row: &[Cell]| row.iter().all(|c| *c == Cell::default());

        if rows < old_rows {
            let mut to_remove = old_rows - rows;
            while to_remove > 0
                && self.cells.len() > self.cursor_y + 1
                && self.cells.last().is_some_and(|r| row_is_blank(r))
            {
                self.cells.pop();
                self.wrapped.pop();
                to_remove -= 1;
            }
            for _ in 0..to_remove {
                if self.cells.is_empty() {
                    break;
                }
                let top = self.cells.remove(0);
                let top_wrapped = if self.wrapped.is_empty() {
                    false
                } else {
                    self.wrapped.remove(0)
                };
                self.scrollback
                    .push_back(ScrollbackLine::new(top, top_wrapped));
                while self.scrollback.len() > self.scrollback_limit {
                    self.scrollback.pop_front();
                }
                self.cursor_y = self.cursor_y.saturating_sub(1);
                match self.last_prompt_end.as_mut() {
                    Some((0, _)) => {
                        self.last_prompt_end = None;
                        self.awaiting_input = false;
                    }
                    Some((r, _)) => *r -= 1,
                    None => {}
                }
            }
            self.cells.truncate(rows);
            self.wrapped.truncate(rows);
        } else {
            let mut to_add = rows - old_rows;
            while to_add > 0 {
                let Some(line) = self.scrollback.pop_back() else {
                    break;
                };
                self.cells.insert(0, line.cells);
                self.wrapped.insert(0, line.wrapped);
                self.cursor_y += 1;
                if let Some((r, _)) = self.last_prompt_end.as_mut() {
                    *r += 1;
                }
                to_add -= 1;
            }
            for _ in 0..to_add {
                self.cells.push(vec![Cell::default(); cols]);
                self.wrapped.push(false);
            }
        }
    }

    /// Re-wrap the scrollback (off-screen history) for a column change.
    ///
    /// `resize` deliberately leaves the LIVE grid untouched (it only clamps row
    /// widths) — re-laying the on-screen lines would move the shell's prompt to
    /// a different row than readline assumes when it repaints on SIGWINCH,
    /// leaving a stale duplicate prompt. Only the history re-flows here, so
    /// scrolled-back long lines stay readable at the new width.
    ///
    /// Runs of soft-wrapped lines are rejoined into logical lines and re-wrapped
    /// at `new_cols`. A still-wrapped final scrollback line continues into grid
    /// row 0; its run is kept open (trailing flag preserved) so that boundary
    /// stays a soft wrap.
    fn rewrap_scrollback(&mut self, new_cols: usize) {
        if self.scrollback.is_empty() {
            return;
        }
        let old: Vec<ScrollbackLine> = self.scrollback.drain(..).collect();
        let last_idx = old.len() - 1;

        // Group consecutive soft-wrapped lines into logical lines. A run ends at
        // the first line whose wrap flag is false; a still-wrapped final line is
        // an open run that continues into the live grid.
        let mut groups: Vec<(Vec<Cell>, bool)> = Vec::new(); // (cells, continues_into_grid)
        let mut cur: Vec<Cell> = Vec::new();
        for (i, line) in old.into_iter().enumerate() {
            cur.extend(line.cells);
            if !line.wrapped {
                groups.push((std::mem::take(&mut cur), false));
            } else if i == last_idx {
                groups.push((std::mem::take(&mut cur), true));
            }
        }
        if !cur.is_empty() {
            groups.push((cur, true));
        }

        // Re-wrap each logical line at the new width and rebuild the scrollback.
        for (cells, open) in groups {
            // Trim trailing blanks on a closed line so a short line doesn't spawn
            // phantom rows; keep an open (grid-continuing) line intact so its
            // boundary cell count survives.
            let content = if open {
                cells.as_slice()
            } else {
                &cells[..trim_trailing_default_len(&cells)]
            };
            let rows = wrap_cells(content, new_cols);
            let n = rows.len();
            for (k, rc) in rows.into_iter().enumerate() {
                let is_last = k + 1 == n;
                let wrapped = if is_last { open } else { true };
                self.scrollback.push_back(ScrollbackLine::new(rc, wrapped));
            }
        }
        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
    }

    /// Return the text content of a grid row with trailing spaces
    /// kept (callers trim as they see fit).
    pub fn line_text(&self, row: usize) -> String {
        self.cells
            .get(row)
            .map(|r| r.iter().map(|c| c.ch).collect())
            .unwrap_or_default()
    }
    /// Check if the current line contains an SSH command and extract
    /// host/user/port. Called when the user presses Enter (LF).
    pub fn detect_ssh_in_current_line(&mut self) {
        let line = self.line_text(self.cursor_y);
        if let Some((host, user, port)) = parse_ssh_command(&line) {
            self.ssh_detected_host = host;
            self.ssh_detected_user = user;
            self.ssh_detected_port = port;
            self.ssh_command_detected = true;
        }
    }
}

/// Parse an SSH command from a terminal line.
///
/// Recognizes: `ssh [-p port] [-i key] [-o opt] [user@]host`
/// Returns `Some((host, user, port))` or `None`.
fn parse_ssh_command(line: &str) -> Option<(String, String, u16)> {
    // Strip shell prompt: find last `$ `, `# `, or `% ` and take everything after
    let cmd_part = line
        .rfind("$ ")
        .or_else(|| line.rfind("# "))
        .or_else(|| line.rfind("% "))
        .map(|pos| &line[pos + 2..])
        .unwrap_or(line)
        .trim();

    let tokens: Vec<&str> = cmd_part.split_whitespace().collect();
    if tokens.is_empty() || tokens[0] != "ssh" {
        return None;
    }

    let mut host = String::new();
    // Empty = "no explicit user given". OpenSSH then uses the local
    // login user (NOT root), and consumers of `ssh_detected_user`
    // are documented to treat empty as "unknown, fall back" — see the
    // field doc above. Defaulting to "root" actively mis-targeted the
    // right-side panels / credential cache to root@host.
    let mut user = String::new();
    let mut port: u16 = 22;

    // Flags that consume the next argument
    let flags_with_arg = [
        "-p", "-i", "-o", "-l", "-L", "-R", "-D", "-F", "-J", "-w", "-W", "-b", "-c", "-E", "-e",
        "-I", "-m", "-O", "-Q", "-S",
    ];

    let mut i = 1; // skip "ssh"
    while i < tokens.len() {
        let t = tokens[i];

        if t == "-p" {
            // Next token is port
            if i + 1 < tokens.len() {
                port = tokens[i + 1].parse().unwrap_or(22);
                i += 2;
                continue;
            }
        } else if t == "-l" {
            // Next token is username
            if i + 1 < tokens.len() {
                user = tokens[i + 1].to_string();
                i += 2;
                continue;
            }
        } else if flags_with_arg.contains(&t) {
            // Skip flag and its argument
            i += 2;
            continue;
        } else if t.starts_with('-') {
            // Skip boolean flags (e.g., -v, -N, -f, -T, -t)
            i += 1;
            continue;
        } else {
            // This should be the [user@]host target
            if let Some(at_pos) = t.find('@') {
                user = t[..at_pos].to_string();
                host = t[at_pos + 1..].to_string();
            } else {
                host = t.to_string();
            }
            break;
        }
        i += 1;
    }

    if host.is_empty() {
        return None;
    }

    Some((host, user, port))
}

// `Default` so `std::mem::take` works in `process`.
impl Default for VtEmulator {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

// ─────────────────────────────────────────────────────────
// vte::Perform implementation — the actual state machine body.
// ─────────────────────────────────────────────────────────

struct Performer<'a> {
    cols: usize,
    rows: usize,
    cursor_x: &'a mut usize,
    cursor_y: &'a mut usize,
    cells: &'a mut Vec<Vec<Cell>>,
    wrapped: &'a mut Vec<bool>,
    scrollback: &'a mut VecDeque<ScrollbackLine>,
    scrollback_limit: usize,
    pen: &'a mut Cell,
    bell_pending: &'a mut bool,
    window_title: &'a mut String,
    osc52_clipboard: &'a mut String,
    cwd: &'a mut String,
    current_user: &'a mut String,
    ssh_command_detected: &'a mut bool,
    ssh_detected_host: &'a mut String,
    ssh_detected_user: &'a mut String,
    ssh_detected_port: &'a mut u16,
    last_prompt_end: &'a mut Option<(usize, usize)>,
    awaiting_input: &'a mut bool,
    alt_screen: &'a mut bool,
    bracketed_paste: &'a mut bool,
    scroll_top: &'a mut usize,
    scroll_bottom: &'a mut usize,
    autowrap: &'a mut bool,
    saved_cursor: &'a mut Option<(usize, usize, Cell)>,
    last_char: &'a mut char,
}

impl Performer<'_> {
    /// Push the top row into scrollback and append a blank row at
    /// the bottom. Called when the cursor would move past the last
    /// visible row.
    fn scroll_up(&mut self) {
        let top = self.cells.remove(0);
        // The evicted row carries its soft-wrap flag into scrollback so a
        // later reflow can still tell where this logical line continued.
        let top_wrapped = if self.wrapped.is_empty() {
            false
        } else {
            self.wrapped.remove(0)
        };
        self.scrollback
            .push_back(ScrollbackLine::new(top, top_wrapped));
        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
        self.cells.push(vec![Cell::default(); self.cols]);
        self.wrapped.push(false);

        // Shift the OSC 133;B prompt-end marker up by one row so the
        // smart-mode UI keeps tracking the still-being-typed line as it
        // scrolls. If the marker was already on the top row it falls
        // off into scrollback and is no longer addressable in the live
        // grid — drop it.
        if let Some((row, _col)) = *self.last_prompt_end {
            if row == 0 {
                *self.last_prompt_end = None;
                *self.awaiting_input = false;
            } else {
                self.last_prompt_end.as_mut().unwrap().0 = row - 1;
            }
        }
    }

    /// LF — move to next row, scrolling if at the bottom margin. Leaves
    /// `cursor_x` alone (that's `\r`'s job, called separately by the
    /// shell's `\r\n` sequence). Honours the DECSTBM scroll region: at
    /// `scroll_bottom` it scrolls the region rather than the whole screen.
    fn line_feed(&mut self) {
        if *self.cursor_y == *self.scroll_bottom {
            self.scroll_up_region();
        } else if *self.cursor_y + 1 < self.rows {
            *self.cursor_y += 1;
        }
    }

    /// Scroll the active region up by one row (content moves up, a blank
    /// row appears at the bottom margin). When the region is the whole
    /// primary screen the evicted top row is preserved in scrollback via
    /// [`Self::scroll_up`]; a sub-region or the alternate screen rotates
    /// rows in place without touching scrollback.
    fn scroll_up_region(&mut self) {
        let top = *self.scroll_top;
        let bottom = *self.scroll_bottom;
        if top == 0 && bottom + 1 == self.rows && !*self.alt_screen {
            self.scroll_up();
            return;
        }
        if top <= bottom && bottom < self.cells.len() {
            self.cells.remove(top);
            self.wrapped.remove(top);
            self.cells.insert(bottom, vec![Cell::default(); self.cols]);
            self.wrapped.insert(bottom, false);
        }
    }

    /// Scroll the active region down by one row (content moves down, a
    /// blank row appears at the top margin). Used by RI (`ESC M`) at the
    /// top margin and by SD (`CSI T`). Never spills into scrollback.
    fn scroll_down_region(&mut self) {
        let top = *self.scroll_top;
        let bottom = *self.scroll_bottom;
        if top <= bottom && bottom < self.cells.len() {
            self.cells.remove(bottom);
            self.wrapped.remove(bottom);
            self.cells.insert(top, vec![Cell::default(); self.cols]);
            self.wrapped.insert(top, false);
        }
    }

    /// RI (Reverse Index) — move up one row, scrolling the region down
    /// when already at the top margin.
    fn reverse_index(&mut self) {
        if *self.cursor_y == *self.scroll_top {
            self.scroll_down_region();
        } else if *self.cursor_y > 0 {
            *self.cursor_y -= 1;
        }
    }
}

impl Perform for Performer<'_> {
    fn print(&mut self, ch: char) {
        // Determine display width of the character.
        // CJK / fullwidth chars take 2 cells; most others take 1.
        let char_width = if is_wide_char(ch) { 2 } else { 1 };
        *self.last_char = ch;

        // Wrap at right edge: if the cursor is past the last column,
        // or a wide char won't fit, wrap to the next line first — but
        // only when autowrap (DECAWM) is on. With it off, the character
        // overwrites the rightmost cell in place; the cursor does not
        // advance past the margin and no scroll is triggered.
        if *self.cursor_x + char_width > self.cols {
            if *self.autowrap {
                // Record that the row we're leaving soft-wrapped into the
                // next one (set before line_feed so a scroll carries the flag
                // along with the row). A hard newline clears it again.
                if let Some(w) = self.wrapped.get_mut(*self.cursor_y) {
                    *w = true;
                }
                *self.cursor_x = 0;
                self.line_feed();
            } else {
                *self.cursor_x = self.cols.saturating_sub(char_width);
            }
        }
        if *self.cursor_y < self.cells.len() && *self.cursor_x < self.cols {
            let mut cell = self.pen.clone();
            cell.ch = ch;
            self.cells[*self.cursor_y][*self.cursor_x] = cell;
            *self.cursor_x += 1;

            // For wide characters, insert a zero-width placeholder in
            // the next cell so the renderer knows to skip it.
            if char_width == 2 && *self.cursor_x < self.cols {
                let placeholder = Cell {
                    ch: '\0',
                    ..Cell::default()
                };
                self.cells[*self.cursor_y][*self.cursor_x] = placeholder;
                *self.cursor_x += 1;
            }
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // LF / VT / FF — all move down one row.
            b'\n' | 0x0B | 0x0C => {
                // Before line feed, check if current line has an SSH command.
                // This detects `ssh user@host` typed by the user.
                let line: String = self.cells[*self.cursor_y].iter().map(|c| c.ch).collect();
                if let Some((host, user, port)) = parse_ssh_command(&line) {
                    *self.ssh_detected_host = host;
                    *self.ssh_detected_user = user;
                    *self.ssh_detected_port = port;
                    *self.ssh_command_detected = true;
                }
                // A hard newline ends the logical line here — the row we
                // are leaving does not soft-wrap into the next.
                if let Some(w) = self.wrapped.get_mut(*self.cursor_y) {
                    *w = false;
                }
                self.line_feed();
                // A real LF (not a print-wrap, which calls `line_feed`
                // internally without going through `execute`) means
                // either the user pressed Enter on the prompt, or the
                // shell is printing output past the prompt row. In
                // both cases the OSC 133;B reference recorded for that
                // prompt is now stale — and crucially, a sub-process
                // that doesn't itself emit OSC 133 (a remote bash
                // reached over `ssh`, a python REPL, `docker exec -it`,
                // …) will never invalidate it through OSC 133;A. If
                // we don't drop it here, the smart-mode UI keeps
                // anchoring its overlay at the local-shell prompt
                // position long after the user has moved on, and
                // every keystroke inside the sub-process gets painted
                // on top of the original prompt line. Drop it now so
                // the overlay only re-arms once a fresh OSC 133;B
                // marks a real prompt.
                if self.last_prompt_end.is_some() {
                    crate::logging::write_event_verbose(
                        "DEBUG",
                        "emu.osc133",
                        &format!(
                            "LF cleared stale prompt_end={:?} cursor=({},{})",
                            *self.last_prompt_end, *self.cursor_y, *self.cursor_x,
                        ),
                    );
                    *self.last_prompt_end = None;
                    *self.awaiting_input = false;
                }
            }
            // CR — back to column 0.
            b'\r' => *self.cursor_x = 0,
            // BS — one column left (but not below 0).
            0x08 => {
                if *self.cursor_x > 0 {
                    *self.cursor_x -= 1;
                }
            }
            // HT — next 8-column tab stop, clamped to last column.
            b'\t' => {
                let next = (*self.cursor_x / 8 + 1) * 8;
                *self.cursor_x = next.min(self.cols - 1);
            }
            // BEL — visual bell. Set the bell flag so the shell can
            // flash the terminal border or play a sound.
            0x07 => {
                *self.bell_pending = true;
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {
    }
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }
        match params[0] {
            // OSC 0 — set icon name + window title
            // OSC 1 — set icon name
            // OSC 2 — set window title
            b"0" | b"1" | b"2" => {
                if params.len() >= 2 {
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        *self.window_title = title.to_string();
                    }
                }
            }
            // OSC 52 — clipboard access (read/write)
            // Security: we store the payload but don't auto-paste.
            // The UI layer decides whether to honor it.
            b"52" => {
                if params.len() >= 3 {
                    if let Ok(data) = std::str::from_utf8(params[2]) {
                        *self.osc52_clipboard = data.to_string();
                    }
                }
            }
            // OSC 7 — shell reports current working directory.
            // Payload is a `file://` URI: `file://hostname/abs/path`.
            // We extract the path (everything after the third `/`)
            // and URL-decode percent escapes. Honoured by
            // default bash/zsh on macOS and common Linux distros.
            b"7" => {
                if params.len() >= 2 {
                    if let Ok(uri) = std::str::from_utf8(params[1]) {
                        if let Some(path) = extract_osc7_path(uri) {
                            *self.cwd = path;
                        }
                    }
                }
            }
            // OSC 9 — multiple Windows-Terminal sub-conventions share
            // this number. Sub-code `9;9;<path>` is "current working
            // directory" (Microsoft's documented native form, used by
            // pwsh / cmd / Tabby / ConEmu). The path may arrive bare
            // (`C:\Users\al`) or double-quoted (`"C:\Users\al"`); we
            // strip the quotes and store the path verbatim.
            //
            // We coexist with OSC 7: whichever sequence the shell
            // emits last wins, which matches Windows Terminal's
            // own behaviour.
            b"9" => {
                if params.len() >= 3 && params[1] == b"9" {
                    if let Ok(raw) = std::str::from_utf8(params[2]) {
                        let trimmed = raw.trim().trim_start_matches('"').trim_end_matches('"');
                        if !trimmed.is_empty() {
                            *self.cwd = trimmed.to_string();
                        }
                    }
                }
            }
            // OSC 1337;CurrentUser=<name> — Pier-X-private prompt
            // metadata emitted by smart.rs. This lets the UI reflect
            // `su root` / `sudo -s` prompt context without treating it
            // as SSH credentials.
            b"1337" => {
                if params.len() >= 2 {
                    if let Ok(raw) = std::str::from_utf8(params[1]) {
                        if let Some(value) = raw.strip_prefix("CurrentUser=") {
                            let trimmed = value.trim();
                            if !trimmed.is_empty()
                                && trimmed.len() <= 128
                                && trimmed.bytes().all(|b| !b.is_ascii_control())
                            {
                                *self.current_user = trimmed.to_string();
                            }
                        }
                    }
                }
            }
            // OSC 133 — prompt-sentinel sequences emitted by smart-mode
            // shells (see smart.rs). We track the cursor position at
            // the moment of `B` (prompt-end) so the UI can overlay the
            // smart layer at the correct cell. `A` (prompt-start) and
            // `C` (command-start) toggle `awaiting_input` so we know
            // whether the cursor is currently inside an editable line.
            // `D` (command-finished) is recorded for symmetry but not
            // currently consumed.
            b"133" => {
                if let Some(kind) = params.get(1).and_then(|p| p.first()) {
                    match *kind {
                        b'A' => {
                            // A new prompt is starting. Any prior B is
                            // stale; reset until the matching B fires.
                            *self.last_prompt_end = None;
                            *self.awaiting_input = false;
                            crate::logging::write_event_verbose(
                                "DEBUG",
                                "emu.osc133",
                                "A reset prompt_end + awaiting_input",
                            );
                        }
                        b'B' => {
                            *self.last_prompt_end = Some((*self.cursor_y, *self.cursor_x));
                            *self.awaiting_input = true;
                            crate::logging::write_event_verbose(
                                "DEBUG",
                                "emu.osc133",
                                &format!(
                                    "B set prompt_end=({},{})",
                                    *self.cursor_y, *self.cursor_x,
                                ),
                            );
                        }
                        b'C' => {
                            // User pressed Enter — the line we were
                            // tracking has been submitted to the shell.
                            *self.awaiting_input = false;
                            crate::logging::write_event_verbose(
                                "DEBUG",
                                "emu.osc133",
                                "C cleared awaiting_input",
                            );
                        }
                        b'D' => {
                            // Command finished. No-op for M1.
                        }
                        _ => {}
                    }
                }
            }
            // OSC 10/11 — default fg/bg color query — silently ignored
            // (responding would require writing back to the PTY, which
            // the emulator doesn't own)
            _ => {}
        }
    }
    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        // Only the intermediate-free ESC sequences concern us. Charset
        // selection (`ESC ( B`, …) and other intermediate forms are no-ops.
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            // IND — Index: move down one row, scrolling the region at the
            // bottom margin. Same downward motion as LF but without a CR.
            b'D' => self.line_feed(),
            // NEL — Next Line: CR + IND.
            b'E' => {
                *self.cursor_x = 0;
                self.line_feed();
            }
            // RI — Reverse Index: move up one row, scrolling the region
            // down at the top margin.
            b'M' => self.reverse_index(),
            // DECSC — save cursor position + pen.
            b'7' => {
                *self.saved_cursor = Some((*self.cursor_x, *self.cursor_y, self.pen.clone()));
            }
            // DECRC — restore the saved cursor + pen (clamped to bounds).
            b'8' => {
                if let Some((x, y, pen)) = self.saved_cursor.clone() {
                    *self.cursor_x = x.min(self.cols - 1);
                    *self.cursor_y = y.min(self.rows - 1);
                    *self.pen = pen;
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        // Private-mode CSI (DECSET / DECRST): `\e[?Nh` / `\e[?Nl`.
        // We need 1049 / 1047 / 47 to know when an alt-screen TUI is
        // running, and 2004 to know when bracketed paste is active.
        // None of the public-CSI handlers below would do anything
        // useful with these, so route them and return.
        if intermediates.first() == Some(&b'?') {
            if action == 'h' || action == 'l' {
                let set = action == 'h';
                for param in params.iter() {
                    let code = param.first().copied().unwrap_or(0);
                    match code {
                        // DECAWM — autowrap at the right margin.
                        7 => *self.autowrap = set,
                        // Alternate screen buffer. On entry we give the TUI
                        // a clean slate (clear grid, home the cursor, drop any
                        // scroll region) the way xterm does — otherwise stale
                        // primary-screen text bleeds through the gaps the TUI
                        // leaves between the cells it paints. We don't keep a
                        // separate primary buffer, so leaving alt only flips the
                        // flag back and resets the region.
                        47 | 1047 | 1049 => {
                            if set && !*self.alt_screen {
                                for row in self.cells.iter_mut() {
                                    row.fill(Cell::default());
                                }
                                for w in self.wrapped.iter_mut() {
                                    *w = false;
                                }
                                *self.cursor_x = 0;
                                *self.cursor_y = 0;
                            }
                            *self.alt_screen = set;
                            *self.scroll_top = 0;
                            *self.scroll_bottom = self.rows - 1;
                        }
                        // Bracketed paste.
                        2004 => *self.bracketed_paste = set,
                        // DECTCEM (cursor visibility) — accepted, not yet
                        // surfaced to the UI; swallow so it isn't printed.
                        25 => {}
                        _ => {}
                    }
                }
            }
            return;
        }
        // Flatten params into a simple Vec<u16> for the common
        // single-value cases. Multi-value params (SGR specifically)
        // iterate the original structure themselves below.
        let flat: Vec<u16> = params
            .iter()
            .map(|p| p.first().copied().unwrap_or(0))
            .collect();
        let first = flat.first().copied().unwrap_or(0);
        let second = flat.get(1).copied().unwrap_or(0);

        match action {
            // CUU — cursor up n (default 1).
            'A' => {
                let n = first.max(1) as usize;
                *self.cursor_y = self.cursor_y.saturating_sub(n);
            }
            // CUD — cursor down n.
            'B' => {
                let n = first.max(1) as usize;
                *self.cursor_y = (*self.cursor_y + n).min(self.rows - 1);
            }
            // CUF — cursor forward n.
            'C' => {
                let n = first.max(1) as usize;
                *self.cursor_x = (*self.cursor_x + n).min(self.cols - 1);
            }
            // CUB — cursor back n.
            'D' => {
                let n = first.max(1) as usize;
                *self.cursor_x = self.cursor_x.saturating_sub(n);
            }
            // CHA / HPA — cursor horizontal absolute (1-based column).
            // ncurses' `hpa` capability; the single commonest way a TUI
            // repositions within a row. Missing it made same-row redraws
            // land at the wrong column (overlapping text).
            'G' | '`' => {
                *self.cursor_x = (first.max(1) as usize - 1).min(self.cols - 1);
            }
            // VPA — vertical position absolute (1-based row). ncurses'
            // `vpa`; missing it made line moves land on the wrong row.
            'd' => {
                *self.cursor_y = (first.max(1) as usize - 1).min(self.rows - 1);
            }
            // CUP / HVP — cursor position row;col (1-based).
            'H' | 'f' => {
                let row = first.max(1) as usize - 1;
                let col = second.max(1) as usize - 1;
                *self.cursor_y = row.min(self.rows - 1);
                *self.cursor_x = col.min(self.cols - 1);
            }
            // ED — erase in display.
            'J' => match first {
                0 => self.erase_display_from_cursor(),
                1 => self.erase_display_to_cursor(),
                2 | 3 => self.erase_display_all(),
                _ => {}
            },
            // EL — erase in line.
            'K' => match first {
                0 => self.erase_line_from_cursor(),
                1 => self.erase_line_to_cursor(),
                2 => self.erase_line_all(),
                _ => {}
            },
            // IL — insert n blank lines at the cursor row, within the
            // scroll region. Rows below shift down; the bottom scrolls off.
            'L' => self.insert_lines(first.max(1) as usize),
            // DL — delete n lines at the cursor row, within the scroll
            // region. Rows below shift up; blanks appear at the bottom.
            'M' => self.delete_lines(first.max(1) as usize),
            // DCH — delete n characters at the cursor, shifting the rest of
            // the line left and padding the right with blanks.
            'P' => self.delete_chars(first.max(1) as usize),
            // ICH — insert n blank characters at the cursor, shifting the
            // rest of the line right (dropping what falls off the margin).
            '@' => self.insert_chars(first.max(1) as usize),
            // ECH — erase n characters from the cursor without shifting.
            'X' => self.erase_chars(first.max(1) as usize),
            // SU / SD — scroll the region up / down by n rows.
            'S' => {
                for _ in 0..first.max(1) {
                    self.scroll_up_region();
                }
            }
            'T' => {
                for _ in 0..first.max(1) {
                    self.scroll_down_region();
                }
            }
            // DECSTBM — set top/bottom scroll margins (1-based, inclusive).
            // An omitted / invalid region resets to the full screen. The
            // cursor homes to the top-left afterwards.
            'r' => {
                let top = first.max(1) as usize - 1;
                let bot = if second == 0 {
                    self.rows
                } else {
                    (second as usize).min(self.rows)
                };
                let bottom = bot.saturating_sub(1);
                if top < bottom && bottom < self.rows {
                    *self.scroll_top = top;
                    *self.scroll_bottom = bottom;
                } else {
                    *self.scroll_top = 0;
                    *self.scroll_bottom = self.rows - 1;
                }
                *self.cursor_x = 0;
                *self.cursor_y = 0;
            }
            // SCP / RCP — save / restore cursor (ANSI.SYS form, mirrors
            // DECSC/DECRC). We don't implement left/right margins, so `s`
            // is unambiguously "save cursor" here.
            's' => {
                *self.saved_cursor = Some((*self.cursor_x, *self.cursor_y, self.pen.clone()));
            }
            'u' => {
                if let Some((x, y, pen)) = self.saved_cursor.clone() {
                    *self.cursor_x = x.min(self.cols - 1);
                    *self.cursor_y = y.min(self.rows - 1);
                    *self.pen = pen;
                }
            }
            // REP — repeat the last printed character n times. ncurses uses
            // this to draw runs of a repeated glyph.
            'b' => {
                let ch = *self.last_char;
                for _ in 0..first.max(1) {
                    self.print(ch);
                }
            }
            // SGR — select graphic rendition. Updates `pen` state
            // that future `print` calls will apply. We handle the
            // subset interactive shells actually emit.
            'm' => self.handle_sgr(params),
            _ => {}
        }
    }
}

// ─────────────────────────────────────────────────────────
// Helpers split out from Performer impl for readability.
// ─────────────────────────────────────────────────────────

impl Performer<'_> {
    fn erase_display_from_cursor(&mut self) {
        // Cursor line: from cursor to end. Clearing the right edge ends
        // any soft-wrap that started on this row.
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            row[*self.cursor_x..].fill(Cell::default());
        }
        if let Some(w) = self.wrapped.get_mut(*self.cursor_y) {
            *w = false;
        }
        // All rows below the cursor.
        for row in self.cells.iter_mut().skip(*self.cursor_y + 1) {
            row.fill(Cell::default());
        }
        for w in self.wrapped.iter_mut().skip(*self.cursor_y + 1) {
            *w = false;
        }
    }

    fn erase_display_to_cursor(&mut self) {
        // All rows above the cursor.
        for row in self.cells.iter_mut().take(*self.cursor_y) {
            row.fill(Cell::default());
        }
        for w in self.wrapped.iter_mut().take(*self.cursor_y) {
            *w = false;
        }
        // Cursor line: from start to cursor inclusive. The right edge is
        // untouched, so the row's wrap flag stays as-is.
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            let end = (*self.cursor_x + 1).min(self.cols);
            row[..end].fill(Cell::default());
        }
    }

    fn erase_display_all(&mut self) {
        for row in self.cells.iter_mut() {
            row.fill(Cell::default());
        }
        for w in self.wrapped.iter_mut() {
            *w = false;
        }
    }

    fn erase_line_from_cursor(&mut self) {
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            row[*self.cursor_x..].fill(Cell::default());
        }
        // Right edge cleared → this row no longer soft-wraps.
        if let Some(w) = self.wrapped.get_mut(*self.cursor_y) {
            *w = false;
        }
    }

    fn erase_line_to_cursor(&mut self) {
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            let end = (*self.cursor_x + 1).min(self.cols);
            row[..end].fill(Cell::default());
        }
    }

    fn erase_line_all(&mut self) {
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            row.fill(Cell::default());
        }
        if let Some(w) = self.wrapped.get_mut(*self.cursor_y) {
            *w = false;
        }
    }

    /// IL — insert `n` blank rows at the cursor row. Only acts when the
    /// cursor is inside the scroll region; rows at the bottom of the region
    /// scroll off. Mirrors xterm: the cursor column resets to 0.
    fn insert_lines(&mut self, n: usize) {
        let y = *self.cursor_y;
        let top = *self.scroll_top;
        let bottom = *self.scroll_bottom;
        if y < top || y > bottom || bottom >= self.cells.len() {
            return;
        }
        let n = n.min(bottom - y + 1);
        for _ in 0..n {
            self.cells.remove(bottom);
            self.wrapped.remove(bottom);
            self.cells.insert(y, vec![Cell::default(); self.cols]);
            self.wrapped.insert(y, false);
        }
        *self.cursor_x = 0;
    }

    /// DL — delete `n` rows at the cursor row within the scroll region.
    /// Rows below shift up and blanks fill in at the bottom margin.
    fn delete_lines(&mut self, n: usize) {
        let y = *self.cursor_y;
        let top = *self.scroll_top;
        let bottom = *self.scroll_bottom;
        if y < top || y > bottom || bottom >= self.cells.len() {
            return;
        }
        let n = n.min(bottom - y + 1);
        for _ in 0..n {
            self.cells.remove(y);
            self.wrapped.remove(y);
            self.cells.insert(bottom, vec![Cell::default(); self.cols]);
            self.wrapped.insert(bottom, false);
        }
        *self.cursor_x = 0;
    }

    /// DCH — delete `n` characters at the cursor, shifting the tail of the
    /// line left and padding the freed cells at the right with blanks.
    fn delete_chars(&mut self, n: usize) {
        let x = *self.cursor_x;
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            if x >= row.len() {
                return;
            }
            let del = n.min(row.len() - x);
            row.drain(x..x + del);
            for _ in 0..del {
                row.push(Cell::default());
            }
        }
    }

    /// ICH — insert `n` blank characters at the cursor, shifting the tail
    /// of the line right; cells pushed past the right margin are dropped.
    fn insert_chars(&mut self, n: usize) {
        let x = *self.cursor_x;
        let cols = self.cols;
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            if x >= row.len() {
                return;
            }
            let ins = n.min(row.len() - x);
            for _ in 0..ins {
                row.insert(x, Cell::default());
            }
            row.truncate(cols);
        }
    }

    /// ECH — erase `n` characters from the cursor rightwards, leaving the
    /// rest of the line where it is (no shift).
    fn erase_chars(&mut self, n: usize) {
        let x = *self.cursor_x;
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            let end = (x + n).min(row.len());
            if x < end {
                row[x..end].fill(Cell::default());
            }
        }
    }

    fn handle_sgr(&mut self, params: &vte::Params) {
        // SGR takes zero or more numeric params. Several of them
        // (38 / 48) are multi-value "extended color" prefixes that
        // consume the next 2 (5;n) or 4 (2;r;g;b) params. We walk the
        // param list linearly rather than flattening because of that.
        let mut iter = params.iter().peekable();

        // A completely empty param list is equivalent to `CSI 0 m`.
        if iter.peek().is_none() {
            *self.pen = Cell::default();
            return;
        }

        while let Some(param) = iter.next() {
            let code = param.first().copied().unwrap_or(0);
            match code {
                0 => *self.pen = Cell::default(),
                1 => self.pen.bold = true,
                4 => self.pen.underline = true,
                7 => self.pen.reverse = true,
                22 => self.pen.bold = false,
                24 => self.pen.underline = false,
                27 => self.pen.reverse = false,
                30..=37 => self.pen.fg = Color::Indexed((code - 30) as u8),
                90..=97 => self.pen.fg = Color::Indexed((code - 90 + 8) as u8),
                40..=47 => self.pen.bg = Color::Indexed((code - 40) as u8),
                100..=107 => self.pen.bg = Color::Indexed((code - 100 + 8) as u8),
                39 => self.pen.fg = Color::Default,
                49 => self.pen.bg = Color::Default,
                38 | 48 => {
                    // Extended-color prefix. Next param is the mode:
                    //   5 → next param is a 256-color index
                    //   2 → next three params are r;g;b
                    let is_fg = code == 38;
                    let Some(mode_p) = iter.next() else { break };
                    let mode = mode_p.first().copied().unwrap_or(0);
                    let color = match mode {
                        5 => {
                            let idx =
                                iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                            Color::Indexed(idx)
                        }
                        2 => {
                            let r = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                            let g = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                            let b = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                            Color::Rgb(r, g, b)
                        }
                        _ => continue,
                    };
                    if is_fg {
                        self.pen.fg = color;
                    } else {
                        self.pen.bg = color;
                    }
                }
                _ => {
                    // Ignore unknown SGR codes rather than blow up.
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────
// Unicode East Asian width detection (subset).
//
// Returns true for characters that occupy two terminal cells.
// This covers CJK Unified Ideographs, Hangul, Katakana,
// fullwidth Latin, and other common double-width ranges.
// A full implementation would use the `unicode-width` crate,
// but this inline table avoids an extra dependency for the
// ranges that matter in practice.
// ─────────────────────────────────────────────────────────

/// Extract the path from an OSC 7 `file://host/path` URI and
/// percent-decode it. Returns `None` if the URI is malformed
/// or the scheme isn't `file`. Used by [`Performer::osc_dispatch`]
/// to learn the shell's current working directory.
fn extract_osc7_path(uri: &str) -> Option<String> {
    // Strip the `file://` prefix. Some shells emit `file:<path>`
    // (no host segment) — accept that too.
    let rest = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("file:"))?;
    // After `file://`, anything up to the next `/` is the host.
    // `file:/path` has no host — rest already starts with `/`.
    let path = if let Some(slash) = rest.find('/') {
        &rest[slash..]
    } else if rest.is_empty() {
        "/"
    } else {
        return None;
    };
    // Percent-decode into raw bytes, THEN interpret as UTF-8. Decoding
    // each `%XX` (or literal byte) straight into a `char` maps bytes
    // 0x80–0xFF to U+0080–U+00FF (Latin-1), so a UTF-8 path like
    // `/srv/caf%C3%A9` (`café`) came out as mojibake `cafÃ©` — which
    // then broke Tab completion / restart-`cd` for any non-ASCII dir.
    // Collecting bytes first lets multi-byte sequences round-trip.
    let mut decoded: Vec<u8> = Vec::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_digit(bytes[i + 1]);
            let lo = hex_digit(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                decoded.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    Some(String::from_utf8_lossy(&decoded).into_owned())
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn is_wide_char(ch: char) -> bool {
    let cp = ch as u32;
    matches!(cp,
        0x1100..=0x115F      // Hangul Jamo
        | 0x2329..=0x232A    // Angle brackets
        | 0x2E80..=0x303E    // CJK Radicals, Kangxi, Ideographic Description
        | 0x3040..=0x33BF    // Hiragana, Katakana, Bopomofo, CJK Compat
        | 0x3400..=0x4DBF    // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF    // CJK Unified Ideographs
        | 0xA000..=0xA4CF    // Yi Syllables and Radicals
        | 0xAC00..=0xD7AF    // Hangul Syllables
        | 0xF900..=0xFAFF    // CJK Compatibility Ideographs
        | 0xFE10..=0xFE6F    // CJK Compatibility Forms, Small Forms
        | 0xFF01..=0xFF60    // Fullwidth Latin, Halfwidth Katakana boundary
        | 0xFFE0..=0xFFE6    // Fullwidth Signs
        | 0x20000..=0x2FFFF  // CJK Extension B, C, D, E, F
        | 0x30000..=0x3FFFF  // CJK Extension G, H
    )
}

/// Length of `cells` with trailing default (blank) cells removed.
fn trim_trailing_default_len(cells: &[Cell]) -> usize {
    let mut n = cells.len();
    while n > 0 && cells[n - 1] == Cell::default() {
        n -= 1;
    }
    n
}

/// Re-wrap a flat run of `cells` into rows of at most `cols`, keeping wide-char
/// pairs (head + `\0` placeholder) together. Always returns at least one row.
fn wrap_cells(cells: &[Cell], cols: usize) -> Vec<Vec<Cell>> {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut cur: Vec<Cell> = Vec::with_capacity(cols);
    let mut i = 0usize;
    while i < cells.len() {
        let ch = cells[i].ch;
        let is_wide_head = ch != '\0' && is_wide_char(ch);
        let need = if is_wide_head { 2 } else { 1 };
        if cur.len() + need > cols && !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
        }
        cur.push(cells[i].clone());
        if is_wide_head {
            if i + 1 < cells.len() && cells[i + 1].ch == '\0' {
                cur.push(cells[i + 1].clone());
                i += 2;
            } else {
                cur.push(Cell {
                    ch: '\0',
                    ..Cell::default()
                });
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    rows.push(cur);
    rows
}

// ─────────────────────────────────────────────────────────
// Tests — deliberately small, deliberately focused on the
// contract the UI relies on. These run in milliseconds.
// ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_plain_text_to_grid() {
        let mut emu = VtEmulator::new(80, 24);
        emu.process(b"Hello, Pier-X!");
        assert_eq!(emu.line_text(0).trim_end(), "Hello, Pier-X!");
        assert_eq!(emu.cursor_x, 14);
        assert_eq!(emu.cursor_y, 0);
    }

    #[test]
    fn cr_lf_moves_to_next_row_column_zero() {
        let mut emu = VtEmulator::new(80, 24);
        emu.process(b"line-1\r\nline-2");
        assert_eq!(emu.line_text(0).trim_end(), "line-1");
        assert_eq!(emu.line_text(1).trim_end(), "line-2");
        assert_eq!(emu.cursor_y, 1);
    }

    #[test]
    fn cursor_position_csi_is_one_based() {
        let mut emu = VtEmulator::new(80, 24);
        // Row 5, col 10, then print X. 1-based ⇒ row index 4, col 9.
        emu.process(b"\x1b[5;10HX");
        assert_eq!(emu.cells[4][9].ch, 'X');
        assert_eq!(emu.cursor_x, 10);
        assert_eq!(emu.cursor_y, 4);
    }

    #[test]
    fn csi_2j_clears_the_whole_screen() {
        let mut emu = VtEmulator::new(80, 24);
        emu.process(b"some text to be wiped");
        emu.process(b"\x1b[2J");
        assert_eq!(emu.line_text(0).trim_end(), "");
    }

    #[test]
    fn sgr_basic_sets_foreground_color() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b[31mred\x1b[0mX");
        assert_eq!(emu.cells[0][0].fg, Color::Indexed(1));
        assert_eq!(emu.cells[0][1].fg, Color::Indexed(1));
        assert_eq!(emu.cells[0][2].fg, Color::Indexed(1));
        // After reset, next char has default attrs.
        assert_eq!(emu.cells[0][3].ch, 'X');
        assert_eq!(emu.cells[0][3].fg, Color::Default);
    }

    #[test]
    fn sgr_truecolor_rgb() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b[38;2;53;116;240mQ");
        assert_eq!(emu.cells[0][0].fg, Color::Rgb(53, 116, 240));
        assert_eq!(emu.cells[0][0].ch, 'Q');
    }

    #[test]
    fn sgr_bold_and_underline() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b[1;4mX\x1b[22mY\x1b[24mZ");
        assert!(emu.cells[0][0].bold);
        assert!(emu.cells[0][0].underline);
        // After CSI 22 (bold off) the Y is no longer bold but still underlined.
        assert!(!emu.cells[0][1].bold);
        assert!(emu.cells[0][1].underline);
        // After CSI 24 (underline off) the Z is plain.
        assert!(!emu.cells[0][2].bold);
        assert!(!emu.cells[0][2].underline);
    }

    #[test]
    fn scroll_past_bottom_pushes_into_scrollback() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"A\r\nB\r\nC\r\nD");
        // Grid was 3 rows. "A" should have scrolled off into the
        // scrollback ring, leaving B/C/D visible.
        assert_eq!(emu.scrollback.len(), 1);
        let evicted: String = emu.scrollback[0].cells.iter().map(|c| c.ch).collect();
        assert_eq!(evicted.trim_end(), "A");
        assert_eq!(emu.line_text(0).trim_end(), "B");
        assert_eq!(emu.line_text(1).trim_end(), "C");
        assert_eq!(emu.line_text(2).trim_end(), "D");
    }

    #[test]
    fn scrollback_limit_is_enforced() {
        let mut emu = VtEmulator::new(4, 2);
        emu.scrollback_limit = 3;
        // Pump 10 lines through a 2-row grid. 8 of them evict; only
        // the most recent 3 should remain in the ring.
        for i in 0..10 {
            emu.process(format!("L{i}\r\n").as_bytes());
        }
        assert_eq!(emu.scrollback.len(), 3);
    }

    #[test]
    fn resize_clamps_cursor_within_new_bounds() {
        let mut emu = VtEmulator::new(80, 24);
        emu.process(b"\x1b[23;79HZ"); // put cursor near the corner
        emu.resize(20, 10);
        assert!(emu.cursor_x < 20);
        assert!(emu.cursor_y < 10);
        assert_eq!(emu.cols, 20);
        assert_eq!(emu.rows, 10);
    }

    #[test]
    fn resize_shrink_then_grow_preserves_rows() {
        let mut emu = VtEmulator::new(20, 4);
        emu.process(b"L0\r\nL1\r\nL2\r\nL3");
        assert_eq!(emu.cursor_y, 3);

        // Shrink: the freshest rows stay live, the rest spill to
        // scrollback instead of being dropped off the bottom.
        emu.resize(20, 2);
        assert_eq!(emu.rows, 2);
        assert_eq!(emu.line_text(0).trim_end(), "L2");
        assert_eq!(emu.line_text(1).trim_end(), "L3");
        assert!(emu.scrollback.len() >= 2);

        // Grow back: the spilled rows return in place — no truncation.
        emu.resize(20, 4);
        assert_eq!(emu.rows, 4);
        assert_eq!(emu.line_text(0).trim_end(), "L0");
        assert_eq!(emu.line_text(1).trim_end(), "L1");
        assert_eq!(emu.line_text(2).trim_end(), "L2");
        assert_eq!(emu.line_text(3).trim_end(), "L3");
    }

    #[test]
    fn resize_shrink_keeps_prompt_when_blank_rows_below_cursor() {
        let mut emu = VtEmulator::new(20, 6);
        emu.process(b"$ "); // prompt on the top row, rows below blank
        assert_eq!(emu.cursor_y, 0);

        emu.resize(20, 3);
        assert_eq!(emu.rows, 3);
        // The prompt is kept; only the blank rows below it were dropped.
        assert_eq!(emu.line_text(0).trim_end(), "$");
        assert_eq!(emu.scrollback.len(), 0);
    }

    #[test]
    fn autowrap_sets_wrapped_flag_hard_newline_clears_it() {
        // Auto-wrap at the right margin marks the row as soft-wrapped.
        let mut emu = VtEmulator::new(5, 3);
        emu.process(b"abcdefg"); // 7 chars at width 5 → wraps after 5
        assert!(emu.wrapped[0], "auto-wrap should mark row 0 wrapped");
        assert!(!emu.wrapped[1], "continuation row is not itself wrapped");

        // A hard newline must NOT mark the row as soft-wrapped.
        let mut emu2 = VtEmulator::new(5, 3);
        emu2.process(b"abc\r\ndef");
        assert!(!emu2.wrapped[0], "hard newline must leave wrapped = false");
    }

    #[test]
    fn resize_cols_rewraps_scrollback_without_loss() {
        let mut emu = VtEmulator::new(10, 2);
        // A 15-char line wraps to two rows; two more printed rows push the whole
        // wrapped line off-screen into scrollback.
        emu.process(b"012345678901234\r\nA\r\nB");
        assert!(
            emu.scrollback.len() >= 2,
            "long line should have spilled to scrollback"
        );
        // Shrink the width — the scrolled-back history must re-wrap, not clip.
        emu.resize(5, 2);
        let mut joined = String::new();
        for line in &emu.scrollback {
            joined.extend(line.cells.iter().map(|c| c.ch));
        }
        assert!(
            joined.replace([' ', '\0'], "").contains("012345678901234"),
            "scrollback re-wrap lost characters: {joined:?}"
        );
    }

    #[test]
    fn resize_cols_keeps_live_prompt_on_its_row() {
        // Anti-duplicate-prompt invariant: a column resize must NOT move the
        // live prompt to a different row (the grid is clamped, not re-wrapped),
        // so the shell's SIGWINCH repaint overwrites it in place.
        let mut emu = VtEmulator::new(20, 5);
        emu.process(b"build ok\r\ntests ok\r\nuser@host:~$ ");
        let row = emu.cursor_y;
        assert!(row > 0, "prompt should sit below the top row");
        assert_eq!(emu.line_text(row).trim_end(), "user@host:~$");

        emu.resize(60, 5); // widen
        assert_eq!(emu.cursor_y, row, "prompt row moved on widen");
        assert_eq!(emu.line_text(row).trim_end(), "user@host:~$");

        emu.resize(30, 5); // narrow (prompt still fits)
        assert_eq!(emu.cursor_y, row, "prompt row moved on narrow");
        assert_eq!(emu.line_text(row).trim_end(), "user@host:~$");
    }

    #[test]
    fn resize_cols_clamps_live_grid_rows() {
        // The live grid is clamped (xterm primary-screen behaviour): an on-screen
        // row wider than the new width is clipped, not re-wrapped, and the cursor
        // stays on its row.
        let mut emu = VtEmulator::new(20, 3);
        emu.process(b"abcdefghij"); // 10 chars on row 0
        emu.resize(6, 3);
        assert_eq!(emu.cols, 6);
        assert_eq!(emu.line_text(0), "abcdef"); // clipped to 6, not re-wrapped
        assert_eq!(emu.cursor_y, 0);
    }

    #[test]
    fn line_wraps_at_right_margin() {
        let mut emu = VtEmulator::new(5, 3);
        emu.process(b"ABCDEFG");
        assert_eq!(emu.line_text(0), "ABCDE");
        assert_eq!(emu.line_text(1).trim_end_matches(' '), "FG");
    }

    #[test]
    fn osc7_sets_cwd_from_file_uri() {
        let mut emu = VtEmulator::new(10, 3);
        // OSC 7 with ST terminator: `\x1b]7;file://host/tmp\x1b\\`
        emu.process(b"\x1b]7;file://localhost/tmp\x1b\\");
        assert_eq!(emu.cwd, "/tmp");
    }

    #[test]
    fn osc7_bell_terminated_sets_cwd() {
        let mut emu = VtEmulator::new(10, 3);
        // Some shells terminate with BEL rather than ST.
        emu.process(b"\x1b]7;file://h/home/user\x07");
        assert_eq!(emu.cwd, "/home/user");
    }

    #[test]
    fn osc7_percent_decodes_path() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b]7;file://h/var/log/my%20dir\x07");
        assert_eq!(emu.cwd, "/var/log/my dir");
    }

    #[test]
    fn osc7_accepts_no_host_variant() {
        let mut emu = VtEmulator::new(10, 3);
        // `file:/path` form (no double-slash). Rare but valid.
        emu.process(b"\x1b]7;file:/srv/app\x07");
        assert_eq!(emu.cwd, "/srv/app");
    }

    #[test]
    fn osc7_ignores_non_file_scheme() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b]7;http://example.com/x\x07");
        assert_eq!(emu.cwd, "");
    }

    #[test]
    fn osc9_9_sets_cwd_for_windows_terminal_pwsh() {
        // Microsoft's pwsh / Windows Terminal convention: OSC 9;9
        // carries a Windows-native path with no `file://` wrapper.
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b]9;9;C:\\Users\\al\x07");
        assert_eq!(emu.cwd, "C:\\Users\\al");
    }

    #[test]
    fn osc9_9_strips_surrounding_quotes() {
        let mut emu = VtEmulator::new(10, 3);
        // Windows Terminal documentation explicitly recommends quoting
        // paths that contain spaces.
        emu.process(b"\x1b]9;9;\"C:\\Program Files\\App\"\x07");
        assert_eq!(emu.cwd, "C:\\Program Files\\App");
    }

    #[test]
    fn osc9_other_subcodes_do_not_overwrite_cwd() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b]7;file://h/before\x07");
        // OSC 9;4 is iTerm2's progress channel — must not be parsed
        // as a cwd.
        emu.process(b"\x1b]9;4;1;50\x07");
        assert_eq!(emu.cwd, "/before");
    }

    #[test]
    fn osc1337_current_user_sets_shell_user() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b]1337;CurrentUser=root\x07");
        assert_eq!(emu.current_user, "root");
    }

    #[test]
    fn osc133_b_records_prompt_end_at_cursor() {
        let mut emu = VtEmulator::new(20, 5);
        // Pretend the shell drew a 4-char prompt "$ X" then emitted
        // OSC 133;A before the prompt and 133;B at the end of it.
        emu.process(b"\x1b]133;A\x07$ X \x1b]133;B\x07");
        assert!(emu.awaiting_input, "B should turn on awaiting_input");
        let (row, col) = emu.last_prompt_end.expect("B set last_prompt_end");
        assert_eq!(row, 0);
        assert_eq!(col, 4); // after "$ X ", four chars in
    }

    #[test]
    fn osc133_a_invalidates_previous_b() {
        let mut emu = VtEmulator::new(20, 5);
        emu.process(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        assert!(emu.awaiting_input);
        // A fresh prompt fires A again — old B is stale.
        emu.process(b"\r\n\x1b]133;A\x07");
        assert!(!emu.awaiting_input);
        assert_eq!(emu.last_prompt_end, None);
    }

    #[test]
    fn lf_invalidates_prompt_end_so_ssh_subshell_does_not_echo_overlay_at_local_prompt() {
        // Reproduction for the "typing inside ssh paints over the
        // local zsh prompt" bug. The local smart-mode shell wraps
        // its prompt with OSC 133;A/B but doesn't emit C; the remote
        // shell reached over `ssh` doesn't emit OSC 133 at all. Without
        // the LF-clears-prompt-end rule, `last_prompt_end` would still
        // point at the local prompt long after the user submitted the
        // ssh command, and the smart UI would happily anchor its
        // overlay there.
        let mut emu = VtEmulator::new(40, 6);
        // Local zsh draws its prompt with the OSC 133 wrappers.
        emu.process(b"\x1b]133;A\x07user@host % \x1b]133;B\x07");
        assert!(emu.awaiting_input);
        assert!(emu.last_prompt_end.is_some());

        // User types `ssh root@host` and hits Enter. The shell echoes
        // the typed command and then a CRLF.
        emu.process(b"ssh root@host\r\n");

        // After the LF, the prompt-end reference must be gone — the
        // remote shell will not emit a fresh OSC 133;A to invalidate
        // it, so the emulator has to.
        assert_eq!(
            emu.last_prompt_end, None,
            "LF after a prompt should drop the OSC 133;B reference",
        );
        assert!(
            !emu.awaiting_input,
            "LF after a prompt should clear awaiting_input",
        );
    }

    #[test]
    fn osc133_c_marks_command_started() {
        let mut emu = VtEmulator::new(20, 5);
        emu.process(b"\x1b]133;A\x07$ \x1b]133;B\x07ls\x1b]133;C\x07");
        assert!(!emu.awaiting_input, "C should clear awaiting_input");
        // last_prompt_end is still recorded; the UI uses awaiting_input
        // to decide whether to mirror further keystrokes.
        assert!(emu.last_prompt_end.is_some());
    }

    #[test]
    fn alt_screen_decset_1049_sets_alt_screen_flag() {
        let mut emu = VtEmulator::new(20, 5);
        assert!(!emu.alt_screen);
        emu.process(b"\x1b[?1049h"); // vim entering alt buffer
        assert!(emu.alt_screen);
        emu.process(b"\x1b[?1049l"); // vim leaving alt buffer
        assert!(!emu.alt_screen);
    }

    #[test]
    fn bracketed_paste_2004_toggles_flag() {
        let mut emu = VtEmulator::new(20, 5);
        emu.process(b"\x1b[?2004h");
        assert!(emu.bracketed_paste);
        emu.process(b"\x1b[?2004l");
        assert!(!emu.bracketed_paste);
    }

    #[test]
    fn scrolling_past_prompt_end_invalidates_it() {
        let mut emu = VtEmulator::new(10, 3);
        // Place a prompt-end marker on row 0.
        emu.process(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        assert_eq!(emu.last_prompt_end, Some((0, 2)));
        // Scroll the grid by enough to push the marked row off the
        // top. `\r\n` once moves us to row 1; another to row 2; a
        // third triggers scroll_up, evicting row 0 into scrollback.
        emu.process(b"\r\nA\r\nB\r\nC");
        assert_eq!(emu.last_prompt_end, None);
        assert!(!emu.awaiting_input);
    }

    #[test]
    fn resize_smaller_invalidates_out_of_bounds_prompt_end() {
        let mut emu = VtEmulator::new(20, 10);
        // Put the prompt-end at (5, 15).
        emu.process(b"\x1b[6;16H\x1b]133;A\x07\x1b]133;B\x07");
        assert!(emu.last_prompt_end.is_some());
        // Shrink so column 15 is no longer addressable.
        emu.resize(10, 10);
        assert_eq!(emu.last_prompt_end, None);
    }

    #[test]
    fn extract_osc7_path_handles_standard_input() {
        assert_eq!(extract_osc7_path("file://host/a/b"), Some("/a/b".into()));
        assert_eq!(extract_osc7_path("file:///root"), Some("/root".into()));
        assert_eq!(extract_osc7_path("file:/root"), Some("/root".into()));
        assert_eq!(
            extract_osc7_path("file://host/my%20dir/app%3Ddb.sqlite"),
            Some("/my dir/app=db.sqlite".into()),
        );
        assert_eq!(extract_osc7_path("https://foo/bar"), None);
    }

    #[test]
    fn cha_moves_to_absolute_column() {
        let mut emu = VtEmulator::new(20, 3);
        emu.process(b"hello");
        // CHA to column 1 (1-based) then overwrite — the classic ncurses
        // "reposition to the changed column and repaint the tail" move.
        emu.process(b"\x1b[1GHELLO");
        assert_eq!(emu.line_text(0).trim_end(), "HELLO");
        // Column 3 (1-based ⇒ index 2).
        emu.process(b"\x1b[3GX");
        assert_eq!(emu.cells[0][2].ch, 'X');
    }

    #[test]
    fn vpa_moves_to_absolute_row() {
        let mut emu = VtEmulator::new(20, 5);
        emu.process(b"\x1b[3d"); // row 3 (1-based ⇒ index 2)
        assert_eq!(emu.cursor_y, 2);
        emu.process(b"Z");
        assert_eq!(emu.cells[2][0].ch, 'Z');
    }

    #[test]
    fn ech_erases_without_shifting() {
        let mut emu = VtEmulator::new(20, 3);
        emu.process(b"abcdef");
        emu.process(b"\x1b[3G"); // to column 3 (index 2 = 'c')
        emu.process(b"\x1b[2X"); // erase 2 chars: c, d
        assert_eq!(emu.cells[0][2].ch, ' ');
        assert_eq!(emu.cells[0][3].ch, ' ');
        assert_eq!(emu.cells[0][4].ch, 'e'); // unshifted
    }

    #[test]
    fn dch_deletes_and_shifts_left() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"abcdef");
        emu.process(b"\x1b[1G"); // column 1
        emu.process(b"\x1b[2P"); // delete "ab"
        assert_eq!(emu.line_text(0).trim_end(), "cdef");
    }

    #[test]
    fn ich_inserts_and_shifts_right() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"abcdef");
        emu.process(b"\x1b[1G"); // column 1
        emu.process(b"\x1b[2@"); // insert 2 blanks
        assert_eq!(emu.cells[0][0].ch, ' ');
        assert_eq!(emu.cells[0][1].ch, ' ');
        assert_eq!(emu.cells[0][2].ch, 'a');
    }

    #[test]
    fn il_dl_move_lines_within_scroll_region() {
        let mut emu = VtEmulator::new(10, 4);
        emu.process(b"L0\r\nL1\r\nL2\r\nL3");
        // Delete the top line: everything below shifts up, blank at bottom.
        emu.process(b"\x1b[1;1H\x1b[1M");
        assert_eq!(emu.line_text(0).trim_end(), "L1");
        assert_eq!(emu.line_text(1).trim_end(), "L2");
        assert_eq!(emu.line_text(2).trim_end(), "L3");
        assert_eq!(emu.line_text(3).trim_end(), "");
        // Insert a blank at the top: content shifts back down.
        emu.process(b"\x1b[1;1H\x1b[1L");
        assert_eq!(emu.line_text(0).trim_end(), "");
        assert_eq!(emu.line_text(1).trim_end(), "L1");
        assert_eq!(emu.line_text(2).trim_end(), "L2");
    }

    #[test]
    fn decstbm_scrolls_only_within_region() {
        // Region rows 2..=3 (1-based) — row 0 is a "status bar" that must
        // stay put while the region below it scrolls. This is exactly the
        // shape nano uses (title bar + scrolling edit area).
        let mut emu = VtEmulator::new(10, 4);
        emu.process(b"TITLE\r\n"); // row 0
        emu.process(b"\x1b[2;4r"); // scroll region = rows 2..4
        emu.process(b"\x1b[2;1Ha\r\nb\r\nc"); // fill the region
        assert_eq!(emu.line_text(1).trim_end(), "a");
        assert_eq!(emu.line_text(3).trim_end(), "c");
        // A newline at the bottom margin scrolls the region, not the screen.
        emu.process(b"\r\nd");
        assert_eq!(emu.line_text(0).trim_end(), "TITLE"); // untouched
        assert_eq!(emu.line_text(1).trim_end(), "b");
        assert_eq!(emu.line_text(2).trim_end(), "c");
        assert_eq!(emu.line_text(3).trim_end(), "d");
        // The status row never spilled into scrollback.
        assert_eq!(emu.scrollback.len(), 0);
    }

    #[test]
    fn reverse_index_scrolls_region_down_at_top_margin() {
        let mut emu = VtEmulator::new(10, 4);
        emu.process(b"\x1b[1;4r"); // full-height region
        emu.process(b"\x1b[1;1Hr0\r\nr1\r\nr2");
        // Cursor to top, then RI: region scrolls down, blank at top.
        emu.process(b"\x1b[1;1H\x1bM");
        assert_eq!(emu.line_text(0).trim_end(), "");
        assert_eq!(emu.line_text(1).trim_end(), "r0");
        assert_eq!(emu.line_text(2).trim_end(), "r1");
    }

    #[test]
    fn decstbm_reset_restores_full_screen_scroll() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b[1;2r"); // region rows 1..2
        emu.process(b"\x1b[r"); // reset to full screen
        assert_eq!(emu.scroll_top, 0);
        assert_eq!(emu.scroll_bottom, 2);
        // Full-screen scroll now spills into scrollback again.
        emu.process(b"A\r\nB\r\nC\r\nD");
        assert_eq!(emu.line_text(0).trim_end(), "B");
        assert!(!emu.scrollback.is_empty());
    }

    #[test]
    fn save_and_restore_cursor_roundtrips() {
        let mut emu = VtEmulator::new(20, 5);
        emu.process(b"\x1b[3;5H"); // row 3 col 5 (indices 2,4)
        emu.process(b"\x1b7"); // DECSC
        emu.process(b"\x1b[1;1Hxyz"); // move + write elsewhere
        emu.process(b"\x1b8"); // DECRC
        assert_eq!(emu.cursor_y, 2);
        assert_eq!(emu.cursor_x, 4);
    }

    #[test]
    fn autowrap_off_overwrites_last_cell_without_scrolling() {
        let mut emu = VtEmulator::new(5, 2);
        emu.process(b"\x1b[?7l"); // DECAWM off
        emu.process(b"ABCDEFGH"); // 8 chars into a width-5 row
        // No wrap, no scroll: everything stayed on row 0, last cell wins.
        assert_eq!(emu.cursor_y, 0);
        assert_eq!(emu.scrollback.len(), 0);
        assert_eq!(emu.cells[0][4].ch, 'H');
        // Re-enable, home the cursor, and confirm wrapping returns.
        emu.process(b"\x1b[?7h\x1b[1;1HABCDEF");
        assert_eq!(emu.line_text(0).trim_end(), "ABCDE");
        assert_eq!(emu.line_text(1).trim_end(), "F");
    }

    #[test]
    fn rep_repeats_last_printed_char() {
        let mut emu = VtEmulator::new(20, 3);
        emu.process(b"-\x1b[4b"); // print '-' then repeat it 4 more times
        assert_eq!(emu.line_text(0).trim_end(), "-----");
    }

    #[test]
    fn alt_screen_entry_clears_grid_and_homes_cursor() {
        let mut emu = VtEmulator::new(20, 3);
        emu.process(b"stale primary text\r\nmore");
        emu.process(b"\x1b[?1049h"); // enter alt screen (nano start)
        assert!(emu.alt_screen);
        assert_eq!(emu.cursor_x, 0);
        assert_eq!(emu.cursor_y, 0);
        assert_eq!(emu.line_text(0).trim_end(), "");
        assert_eq!(emu.line_text(1).trim_end(), "");
    }

    #[test]
    fn alt_screen_scroll_does_not_pollute_scrollback() {
        let mut emu = VtEmulator::new(10, 3);
        emu.process(b"\x1b[?1049h");
        // Fill and overflow the alt screen — an alt-screen TUI's scroll must
        // never leak into the primary scrollback ring.
        emu.process(b"A\r\nB\r\nC\r\nD\r\nE");
        assert!(emu.alt_screen);
        assert_eq!(emu.scrollback.len(), 0);
    }
}
