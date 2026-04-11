//! VT100 / ANSI terminal emulator.
//!
//! Thin wrapper over the `vte` crate's state machine. We own the grid
//! (rows × cols of [`Cell`]), a cursor position, the current SGR
//! attributes (foreground / background color, bold, underline,
//! reverse), and a bounded scrollback buffer. Bytes produced by a
//! [`super::Pty`] are fed in via [`VtEmulator::process`]; the UI layer
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

/// Terminal color. The parser distinguishes three variants so the UI
/// can implement its theme's palette lookup the way it prefers.
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
        };
        parser.advance(&mut performer, bytes);
        self.parser = parser;
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
    }

    /// Return the text content of a grid row with trailing spaces
    /// kept (callers trim as they see fit).
    pub fn line_text(&self, row: usize) -> String {
        self.cells
            .get(row)
            .map(|r| r.iter().map(|c| c.ch).collect())
            .unwrap_or_default()
    }
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
        // Wrap at right edge: commit the character into the last
        // column first, then advance cursor past cols so the next
        // print triggers a line feed.
        if *self.cursor_x >= self.cols {
            *self.cursor_x = 0;
            self.line_feed();
        }
        if *self.cursor_y < self.cells.len() && *self.cursor_x < self.cols {
            let mut cell = self.pen.clone();
            cell.ch = ch;
            self.cells[*self.cursor_y][*self.cursor_x] = cell;
            *self.cursor_x += 1;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // LF / VT / FF — all move down one row.
            b'\n' | 0x0B | 0x0C => self.line_feed(),
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
            // BEL — ignored. TODO: emit a visual-bell event in M2b.
            0x07 => {}
            _ => {}
        }
    }

    fn hook(&mut self, _params: &vte::Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // TODO: OSC 0/1/2 (window title), OSC 52 (clipboard),
        // OSC 10/11 (default fg/bg query) — all land in M2b.
    }
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, _byte: u8) {}

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
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
                            let idx = iter
                                .next()
                                .and_then(|p| p.first().copied())
                                .unwrap_or(0) as u8;
                            Color::Indexed(idx)
                        }
                        2 => {
                            let r = iter
                                .next()
                                .and_then(|p| p.first().copied())
                                .unwrap_or(0) as u8;
                            let g = iter
                                .next()
                                .and_then(|p| p.first().copied())
                                .unwrap_or(0) as u8;
                            let b = iter
                                .next()
                                .and_then(|p| p.first().copied())
                                .unwrap_or(0) as u8;
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
            emu.process(format!("L{}\r\n", i).as_bytes());
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
}
