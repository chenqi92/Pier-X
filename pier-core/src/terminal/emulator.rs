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
//! * CSI cursor movement `A B C D H f`
//! * CSI erase `J` and `K` (0/1/2 variants)
//! * CSI `m` — SGR, enough of it to set fg/bg/bold/underline/reverse
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
pub type ScrollbackLine = Vec<Cell>;

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
            scrollback: VecDeque::new(),
            scrollback_limit: 10_000,
            pen: Cell::default(),
            bell_pending: false,
            window_title: String::new(),
            osc52_clipboard: String::new(),
            cwd: String::new(),
            ssh_command_detected: false,
            ssh_detected_host: String::new(),
            ssh_detected_user: String::new(),
            ssh_detected_port: 22,
            ssh_exit_detected: false,
            last_prompt_end: None,
            awaiting_input: false,
            alt_screen: false,
            bracketed_paste: false,
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
            scrollback: &mut self.scrollback,
            scrollback_limit: self.scrollback_limit,
            pen: &mut self.pen,
            bell_pending: &mut self.bell_pending,
            window_title: &mut self.window_title,
            osc52_clipboard: &mut self.osc52_clipboard,
            cwd: &mut self.cwd,
            ssh_command_detected: &mut self.ssh_command_detected,
            ssh_detected_host: &mut self.ssh_detected_host,
            ssh_detected_user: &mut self.ssh_detected_user,
            ssh_detected_port: &mut self.ssh_detected_port,
            last_prompt_end: &mut self.last_prompt_end,
            awaiting_input: &mut self.awaiting_input,
            alt_screen: &mut self.alt_screen,
            bracketed_paste: &mut self.bracketed_paste,
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

    /// Resize the grid. If the new size is smaller, rows are trimmed
    /// off the bottom and extra columns truncated. If larger, blank
    /// cells are appended. The cursor is clamped inside the new grid.
    ///
    /// This is intentionally simple — it does NOT reflow wrapped
    /// lines the way alacritty/kitty do. That's M2b+ work.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == 0 || rows == 0 {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.cells.resize(rows, vec![Cell::default(); cols]);
        for row in self.cells.iter_mut() {
            row.resize(cols, Cell::default());
        }
        if self.cursor_x >= cols {
            self.cursor_x = cols - 1;
        }
        if self.cursor_y >= rows {
            self.cursor_y = rows - 1;
        }
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
    let mut user = String::from("root");
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
    scrollback: &'a mut VecDeque<ScrollbackLine>,
    scrollback_limit: usize,
    pen: &'a mut Cell,
    bell_pending: &'a mut bool,
    window_title: &'a mut String,
    osc52_clipboard: &'a mut String,
    cwd: &'a mut String,
    ssh_command_detected: &'a mut bool,
    ssh_detected_host: &'a mut String,
    ssh_detected_user: &'a mut String,
    ssh_detected_port: &'a mut u16,
    last_prompt_end: &'a mut Option<(usize, usize)>,
    awaiting_input: &'a mut bool,
    alt_screen: &'a mut bool,
    bracketed_paste: &'a mut bool,
}

impl Performer<'_> {
    /// Push the top row into scrollback and append a blank row at
    /// the bottom. Called when the cursor would move past the last
    /// visible row.
    fn scroll_up(&mut self) {
        let top = self.cells.remove(0);
        self.scrollback.push_back(top);
        while self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
        self.cells.push(vec![Cell::default(); self.cols]);

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

    /// LF — move to next row, scrolling if at the bottom. Leaves
    /// `cursor_x` alone (that's `\r`'s job, called separately by the
    /// shell's `\r\n` sequence).
    fn line_feed(&mut self) {
        if *self.cursor_y + 1 >= self.rows {
            self.scroll_up();
        } else {
            *self.cursor_y += 1;
        }
    }
}

impl Perform for Performer<'_> {
    fn print(&mut self, ch: char) {
        // Determine display width of the character.
        // CJK / fullwidth chars take 2 cells; most others take 1.
        let char_width = if is_wide_char(ch) { 2 } else { 1 };

        // Wrap at right edge: if the cursor is past the last column,
        // or a wide char won't fit, wrap to the next line first.
        if *self.cursor_x + char_width > self.cols {
            *self.cursor_x = 0;
            self.line_feed();
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
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}

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
                        47 | 1047 | 1049 => *self.alt_screen = set,
                        2004 => *self.bracketed_paste = set,
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
        // Cursor line: from cursor to end.
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            row[*self.cursor_x..].fill(Cell::default());
        }
        // All rows below the cursor.
        for row in self.cells.iter_mut().skip(*self.cursor_y + 1) {
            row.fill(Cell::default());
        }
    }

    fn erase_display_to_cursor(&mut self) {
        // All rows above the cursor.
        for row in self.cells.iter_mut().take(*self.cursor_y) {
            row.fill(Cell::default());
        }
        // Cursor line: from start to cursor inclusive.
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            let end = (*self.cursor_x + 1).min(self.cols);
            row[..end].fill(Cell::default());
        }
    }

    fn erase_display_all(&mut self) {
        for row in self.cells.iter_mut() {
            row.fill(Cell::default());
        }
    }

    fn erase_line_from_cursor(&mut self) {
        if let Some(row) = self.cells.get_mut(*self.cursor_y) {
            row[*self.cursor_x..].fill(Cell::default());
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
    // Percent-decode. Keep the decoder dependency-free — only
    // `%XX` hex escapes need handling for filesystem paths.
    let mut out = String::with_capacity(path.len());
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_digit(bytes[i + 1]);
            let lo = hex_digit(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    Some(out)
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
        let evicted: String = emu.scrollback[0].iter().map(|c| c.ch).collect();
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
}
