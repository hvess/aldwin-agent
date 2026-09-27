//! A terminal emulator cut down to what ratatui emits, producing the declared
//! cell grid (character, foreground, background) that quiesce waits on and
//! stage 8 reads positions from; pixels cannot give declared colours.
//!
//! Handles cursor addressing, erases, SGR, DECTCEM and the alternate screen.
//! Anything else is skipped, never guessed: an invented cell would mislead
//! the judge. `proxy::verify_against_pixels` cross-checks the result.

use unicode_width::UnicodeWidthChar;

/// A colour as the app declared it in SGR.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Color {
    /// The terminal's default: the app never painted this cell. Inside a
    /// frame this is a design finding.
    #[default]
    Default,
    /// One of the terminal's 256 palette entries; foot picks the colour.
    Indexed(u8),
    /// A truecolour value, how every design token reaches the terminal.
    Rgb(u8, u8, u8),
}

/// The SGR attributes a cell was drawn with.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Attrs {
    /// SGR 1.
    pub bold: bool,
    /// SGR 2.
    pub dim: bool,
    /// SGR 3.
    pub italic: bool,
    /// SGR 4.
    pub underline: bool,
    /// SGR 7: foreground and background swapped; see [`Cell::effective`].
    pub reverse: bool,
}

/// One terminal cell as the app declared it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Cell {
    /// The character, or `'\0'` for the right half of a double-width glyph.
    pub ch: char,
    /// Foreground as written, before `reverse` is applied.
    pub fg: Color,
    /// Background as written, before `reverse` is applied.
    pub bg: Color,
    /// The attributes in force when the cell was written.
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attrs::default(),
        }
    }
}

impl Cell {
    /// The `(fg, bg)` pair shown, with SGR 7 applied.
    pub fn effective(&self) -> (Color, Color) {
        if self.attrs.reverse {
            (self.bg, self.fg)
        } else {
            (self.fg, self.bg)
        }
    }

    /// Whether this is the right half of a double-width glyph.
    pub fn is_continuation(&self) -> bool {
        self.ch == '\0'
    }
}

/// The screen the app has drawn: every cell, row-major, and the caret.
#[derive(Clone, Debug)]
pub struct Grid {
    /// Width in cells.
    pub cols: u16,
    /// Height in cells.
    pub rows: u16,
    cells: Vec<Cell>,
    /// The shown cursor's `(row, col)`: the app's caret, which no cell carries.
    caret: Option<(u16, u16)>,
}

impl Grid {
    fn new(cols: u16, rows: u16) -> Self {
        Grid {
            cols,
            rows,
            cells: vec![Cell::default(); cols as usize * rows as usize],
            caret: None,
        }
    }

    /// The shown cursor's `(row, col)`, or `None` while it is hidden.
    pub fn caret(&self) -> Option<(u16, u16)> {
        self.caret
    }

    /// The cell at `row`, `col`; a default, unpainted cell outside the grid.
    pub fn get(&self, row: u16, col: u16) -> Cell {
        if row >= self.rows || col >= self.cols {
            return Cell::default();
        }
        self.cells[row as usize * self.cols as usize + col as usize]
    }

    fn at_mut(&mut self, row: u16, col: u16) -> &mut Cell {
        let i = row as usize * self.cols as usize + col as usize;
        &mut self.cells[i]
    }

    /// One row's characters, continuation halves skipped and trailing
    /// spaces trimmed.
    pub fn row_text(&self, row: u16) -> String {
        (0..self.cols)
            .map(|c| self.get(row, c))
            .filter(|c| !c.is_continuation())
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Every row as [`Grid::row_text`], newline-joined: the `.txt` beside each
    /// frame.
    pub fn text(&self) -> String {
        (0..self.rows)
            .map(|r| self.row_text(r))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A hash of the visible state (cells and caret), which quiesce compares.
    ///
    /// Never quiesce on bytes instead: ratatui emits a frame envelope
    /// (synchronised-update markers, cursor hide/show, SGR reset) every tick,
    /// so the byte stream never stops.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.cells.hash(&mut hasher);
        self.caret.hash(&mut hasher);
        hasher.finish()
    }

    /// Every cell with its `(row, col)`, row-major.
    pub fn cells(&self) -> impl Iterator<Item = (u16, u16, Cell)> + '_ {
        (0..self.rows).flat_map(move |r| (0..self.cols).map(move |c| (r, c, self.get(r, c))))
    }
}

#[derive(Debug, PartialEq)]
enum State {
    Ground,
    Esc,
    Csi,
    Osc,
    Dcs,
    Skip1,
}

/// The parser: feed it the app's bytes and read the [`Grid`] they drew.
#[derive(Debug)]
pub struct Vt {
    grid: Grid,
    row: u16,
    col: u16,
    pen: Cell,
    state: State,
    params: Vec<u32>,
    current: Option<u32>,
    private: bool,
    /// A `:` was seen in this CSI; the only way to tell `38:2::r:g:b` from
    /// `38;2;0;g;b` followed by more parameters.
    colons: bool,
    utf8: Vec<u8>,
    need: usize,
    /// DECTCEM: whether the cursor is shown; starts on.
    shown: bool,
}

impl Vt {
    /// A blank screen of `cols` × `rows` with the cursor home and shown.
    pub fn new(cols: u16, rows: u16) -> Self {
        Vt {
            grid: Grid::new(cols, rows),
            row: 0,
            col: 0,
            pen: Cell::default(),
            state: State::Ground,
            params: Vec::new(),
            current: None,
            private: false,
            colons: false,
            utf8: Vec::new(),
            need: 0,
            shown: true,
        }
    }

    /// The screen as the bytes fed so far have left it.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Parses `bytes` into the grid. A sequence may be split across calls;
    /// the parser's state carries over.
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b);
        }
        // Read after the whole chunk: a frame hides the cursor, paints, then
        // shows it at the caret.
        self.grid.caret = self.shown.then_some((self.row, self.col));
    }

    fn byte(&mut self, b: u8) {
        match self.state {
            State::Skip1 => self.state = State::Ground,
            State::Osc => {
                // Ends at BEL, or ESC \ (the backslash lands in Skip1).
                if b == 0x07 {
                    self.state = State::Ground;
                } else if b == 0x1b {
                    self.state = State::Skip1;
                }
            }
            State::Dcs => {
                if b == 0x1b {
                    self.state = State::Skip1;
                }
            }
            State::Esc => match b {
                b'[' => {
                    self.params.clear();
                    self.current = None;
                    self.private = false;
                    self.colons = false;
                    self.state = State::Csi;
                }
                b']' => self.state = State::Osc,
                b'P' | b'X' | b'^' | b'_' => self.state = State::Dcs,
                b'(' | b')' | b'*' | b'+' | b'#' => self.state = State::Skip1,
                _ => self.state = State::Ground,
            },
            State::Csi => match b {
                b'0'..=b'9' => {
                    let d = (b - b'0') as u32;
                    self.current = Some(
                        self.current
                            .unwrap_or(0)
                            .saturating_mul(10)
                            .saturating_add(d),
                    );
                }
                // Colons separate too, as in `38:2::r:g:b`.
                b';' | b':' => {
                    self.colons |= b == b':';
                    let v = self.current.take().unwrap_or(0);
                    self.params.push(v);
                }
                b'?' | b'<' | b'=' | b'>' => self.private = true,
                0x20..=0x2f => {}
                0x40..=0x7e => {
                    if let Some(v) = self.current.take() {
                        self.params.push(v);
                    }
                    self.csi(b);
                    self.state = State::Ground;
                }
                // ESC abandons this sequence and starts the next; dropping it
                // would print the next sequence's body as text.
                0x1b => self.state = State::Esc,
                _ => self.state = State::Ground,
            },
            State::Ground => {
                if b == 0x1b {
                    self.utf8.clear();
                    self.need = 0;
                    self.state = State::Esc;
                } else if b < 0x20 || b == 0x7f {
                    self.execute(b);
                } else {
                    self.utf8_byte(b);
                }
            }
        }
    }

    fn utf8_byte(&mut self, b: u8) {
        let continuation = b & 0xc0 == 0x80;
        // A truncated sequence is dropped; waiting for its missing bytes
        // would swallow the valid text after it.
        if self.need != 0 && !continuation {
            self.need = 0;
        }
        if self.need == 0 {
            self.need = match b {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                0xf0..=0xf7 => 4,
                // A stray continuation or an impossible lead: skipped.
                _ => return,
            };
            self.utf8.clear();
        }
        self.utf8.push(b);
        if self.utf8.len() >= self.need {
            if let Ok(s) = std::str::from_utf8(&self.utf8) {
                if let Some(ch) = s.chars().next() {
                    self.print(ch);
                }
            }
            self.utf8.clear();
            self.need = 0;
        }
    }

    fn execute(&mut self, b: u8) {
        match b {
            0x08 => self.col = self.col.saturating_sub(1),
            // Clamped to the last column: unclamped, a run of tabs overflows
            // the u16 `col`.
            0x09 => self.col = (((self.col / 8) + 1) * 8).min(self.grid.cols.saturating_sub(1)),
            0x0a..=0x0c => self.line_feed(),
            0x0d => self.col = 0,
            _ => {}
        }
    }

    fn line_feed(&mut self) {
        if self.row + 1 >= self.grid.rows {
            self.scroll_up();
        } else {
            self.row += 1;
        }
    }

    fn scroll_up(&mut self) {
        let w = self.grid.cols as usize;
        let blank = self.blank();
        self.grid.cells.copy_within(w.., 0);
        let last = self.grid.cells.len() - w;
        self.grid.cells[last..].fill(blank);
    }

    fn print(&mut self, ch: char) {
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            return;
        }
        if self.col >= self.grid.cols {
            self.col = 0;
            self.line_feed();
        }
        let (row, col) = (self.row, self.col);
        let pen = self.pen;
        *self.grid.at_mut(row, col) = Cell { ch, ..pen };
        if width == 2 && col + 1 < self.grid.cols {
            *self.grid.at_mut(row, col + 1) = Cell { ch: '\0', ..pen };
        }
        self.col = self.col.saturating_add(width as u16);
    }

    /// A 1-based row parameter, clamped into the grid.
    fn clamp_row(&self, param: u32) -> u16 {
        (param.max(1) - 1).min(self.grid.rows.saturating_sub(1) as u32) as u16
    }

    /// A 1-based column parameter, clamped into the grid.
    fn clamp_col(&self, param: u32) -> u16 {
        (param.max(1) - 1).min(self.grid.cols.saturating_sub(1) as u32) as u16
    }

    /// Parameter `i`; missing or zero means `default`.
    fn param(&self, i: usize, default: u32) -> u32 {
        self.params
            .get(i)
            .copied()
            .filter(|&v| v != 0)
            .unwrap_or(default)
    }

    /// A relative move's count, clamped: `as u16` would wrap `CSI 65536 B` to 0.
    fn count(&self) -> u16 {
        self.param(0, 1).min(u16::MAX as u32) as u16
    }

    fn csi(&mut self, final_byte: u8) {
        match final_byte {
            b'H' | b'f' => {
                self.row = self.clamp_row(self.param(0, 1));
                self.col = self.clamp_col(self.param(1, 1));
            }
            // Every move saturates and clamps; the app is untrusted. An
            // overflow or an out-of-range row (indexed by `erase_line`) would
            // panic the pump thread and poison the parser's lock.
            b'A' => self.row = self.row.saturating_sub(self.count()),
            b'B' => {
                self.row = self
                    .row
                    .saturating_add(self.count())
                    .min(self.grid.rows.saturating_sub(1))
            }
            b'C' => {
                self.col = self
                    .col
                    .saturating_add(self.count())
                    .min(self.grid.cols.saturating_sub(1))
            }
            b'D' => self.col = self.col.saturating_sub(self.count()),
            b'G' => self.col = self.clamp_col(self.param(0, 1)),
            b'd' => self.row = self.clamp_row(self.param(0, 1)),
            b'J' => self.erase_display(self.param(0, 0)),
            b'K' => self.erase_line(self.param(0, 0)),
            b'X' => {
                let n = self.count();
                let (row, col) = (self.row, self.col);
                for c in col..col.saturating_add(n).min(self.grid.cols) {
                    *self.grid.at_mut(row, c) = self.blank();
                }
            }
            b'm' => self.sgr(),
            // DECTCEM (`?25`): the caret's blink.
            b'h' | b'l' if self.private && self.params.first() == Some(&25) => {
                self.shown = final_byte == b'h';
            }
            // `?1049` alternate screen: modelled as a blank grid either way.
            b'h' | b'l' if self.private && self.params.first() == Some(&1049) => {
                self.grid = Grid::new(self.grid.cols, self.grid.rows);
                self.row = 0;
                self.col = 0;
            }
            _ => {}
        }
    }

    fn blank(&self) -> Cell {
        Cell {
            ch: ' ',
            fg: self.pen.fg,
            bg: self.pen.bg,
            attrs: Attrs::default(),
        }
    }

    fn erase_display(&mut self, mode: u32) {
        let blank = self.blank();
        let (rows, cols) = (self.grid.rows, self.grid.cols);
        let (row, col) = (self.row, self.col);
        match mode {
            0 => {
                for c in col..cols {
                    *self.grid.at_mut(row, c) = blank;
                }
                for r in (row + 1)..rows {
                    for c in 0..cols {
                        *self.grid.at_mut(r, c) = blank;
                    }
                }
            }
            1 => {
                for r in 0..row {
                    for c in 0..cols {
                        *self.grid.at_mut(r, c) = blank;
                    }
                }
                for c in 0..col.saturating_add(1).min(cols) {
                    *self.grid.at_mut(row, c) = blank;
                }
            }
            _ => {
                for r in 0..rows {
                    for c in 0..cols {
                        *self.grid.at_mut(r, c) = blank;
                    }
                }
            }
        }
    }

    fn erase_line(&mut self, mode: u32) {
        let blank = self.blank();
        let (row, col, cols) = (self.row, self.col, self.grid.cols);
        let range = match mode {
            0 => col..cols,
            1 => 0..col.saturating_add(1).min(cols),
            _ => 0..cols,
        };
        for c in range {
            *self.grid.at_mut(row, c) = blank;
        }
    }

    fn sgr(&mut self) {
        if self.params.is_empty() {
            self.pen = Cell::default();
            return;
        }
        let mut i = 0;
        while i < self.params.len() {
            match self.params[i] {
                0 => self.pen = Cell::default(),
                1 => self.pen.attrs.bold = true,
                2 => self.pen.attrs.dim = true,
                3 => self.pen.attrs.italic = true,
                4 => self.pen.attrs.underline = true,
                7 => self.pen.attrs.reverse = true,
                22 => {
                    self.pen.attrs.bold = false;
                    self.pen.attrs.dim = false;
                }
                23 => self.pen.attrs.italic = false,
                24 => self.pen.attrs.underline = false,
                27 => self.pen.attrs.reverse = false,
                30..=37 => self.pen.fg = Color::Indexed((self.params[i] - 30) as u8),
                39 => self.pen.fg = Color::Default,
                40..=47 => self.pen.bg = Color::Indexed((self.params[i] - 40) as u8),
                49 => self.pen.bg = Color::Default,
                90..=97 => self.pen.fg = Color::Indexed((self.params[i] - 90 + 8) as u8),
                100..=107 => self.pen.bg = Color::Indexed((self.params[i] - 100 + 8) as u8),
                n @ (38 | 48 | 58) => {
                    let (color, consumed) = extended(&self.params[i + 1..], self.colons);
                    match (n, color) {
                        (38, Some(c)) => self.pen.fg = c,
                        (48, Some(c)) => self.pen.bg = c,
                        _ => {} // 58 is the underline colour; nothing reads it
                    }
                    i += consumed;
                }
                _ => {}
            }
            i += 1;
        }
    }
}

/// Parses `38;5;n` and `38;2;r;g;b`, returning how many extra parameters were
/// consumed. The 5-parameter `38:2::r:g:b` is accepted only when `colons`:
/// crossterm sends `38;2;0;g;b;48;…`, where a red of 0 would otherwise eat
/// the background.
fn extended(rest: &[u32], colons: bool) -> (Option<Color>, usize) {
    match rest {
        [5, n, ..] => (Some(Color::Indexed(*n as u8)), 2),
        [2, 0, r, g, b, ..] if colons => (Some(Color::Rgb(*r as u8, *g as u8, *b as u8)), 5),
        [2, r, g, b, ..] => (Some(Color::Rgb(*r as u8, *g as u8, *b as u8)), 4),
        [2 | 5, ..] => (None, rest.len()),
        _ => (None, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vt(bytes: &[u8]) -> Vt {
        let mut vt = Vt::new(20, 5);
        vt.feed(bytes);
        vt
    }

    #[test]
    fn absolute_positioning_places_a_row_where_the_app_asked() {
        // ratatui addresses every row with CUP.
        let vt = vt(b"\x1b[3;5Hhello");
        assert_eq!(vt.grid().row_text(2), "    hello");
        assert_eq!(vt.grid().row_text(0), "");
    }

    #[test]
    fn truecolor_foreground_and_background_are_carried_per_cell() {
        let vt = vt(b"\x1b[38;2;190;157;247m\x1b[48;2;39;35;47mx");
        let cell = vt.grid().get(0, 0);
        assert_eq!(cell.ch, 'x');
        assert_eq!(cell.fg, Color::Rgb(190, 157, 247));
        assert_eq!(cell.bg, Color::Rgb(39, 35, 47));
    }

    #[test]
    fn a_cell_the_app_never_painted_keeps_the_default_colour() {
        let vt = vt(b"\x1b[1;1Hx");
        assert_eq!(vt.grid().get(0, 1).bg, Color::Default);
    }

    #[test]
    fn reverse_video_swaps_the_pair_the_eye_actually_gets() {
        let vt = vt(b"\x1b[38;2;1;2;3m\x1b[48;2;4;5;6m\x1b[7mx");
        let (fg, bg) = vt.grid().get(0, 0).effective();
        assert_eq!(fg, Color::Rgb(4, 5, 6));
        assert_eq!(bg, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn erase_in_line_clears_to_the_end_with_the_current_ground() {
        let mut vt = vt(b"abcdef\x1b[1;3H\x1b[48;2;9;9;9m\x1b[K");
        vt.feed(b"");
        assert_eq!(vt.grid().row_text(0), "ab");
        assert_eq!(vt.grid().get(0, 4).bg, Color::Rgb(9, 9, 9));
    }

    #[test]
    fn entering_the_alternate_screen_starts_from_a_blank_grid() {
        let vt = vt(b"leftovers\x1b[?1049h");
        assert_eq!(vt.grid().text().trim(), "");
    }

    #[test]
    fn a_double_width_glyph_occupies_two_cells() {
        // Must use the same `unicode-width` crate as the TUI.
        let vt = vt("漢x".as_bytes());
        assert_eq!(vt.grid().get(0, 0).ch, '漢');
        assert!(vt.grid().get(0, 1).is_continuation());
        assert_eq!(vt.grid().get(0, 2).ch, 'x');
        assert_eq!(vt.grid().row_text(0), "漢x");
    }

    #[test]
    fn a_cursor_move_past_the_grid_cannot_panic_the_parser() {
        // A panic here would poison the pump thread's lock.
        let mut vt = Vt::new(20, 5);
        vt.feed(b"\x1b[99d\x1b[99Gx\x1b[2K");
        // Relative moves: u16 overflow.
        vt.feed(b"\x1b[65535B\x1b[65535C\x1b[65535X");
        assert_eq!(vt.grid().rows, 5);
    }

    #[test]
    fn a_foreground_with_no_red_does_not_eat_the_background_set_beside_it() {
        // crossterm's `SetColors` is one sequence.
        let vt = vt(b"\x1b[38;2;0;10;20;48;2;4;5;6mx");
        let cell = vt.grid().get(0, 0);
        assert_eq!(cell.fg, Color::Rgb(0, 10, 20));
        assert_eq!(cell.bg, Color::Rgb(4, 5, 6));
        assert!(!cell.attrs.dim, "the background's `2` was read as SGR dim");
    }

    #[test]
    fn the_colon_spelling_keeps_its_empty_colour_space_slot() {
        let vt = vt(b"\x1b[38:2::1:2:3mx");
        assert_eq!(vt.grid().get(0, 0).fg, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn a_broken_utf8_sequence_does_not_swallow_the_text_after_it() {
        // A lead byte with no continuation, then a stray continuation.
        let vt = vt(b"\xe6ab\x80cd");
        assert_eq!(vt.grid().row_text(0), "abcd");
    }

    #[test]
    fn a_run_of_tabs_stops_at_the_last_column() {
        let mut vt = Vt::new(20, 5);
        vt.feed(&[b'\t'; 9000]);
        vt.feed(b"\x1b[1K");
        assert_eq!(vt.grid().cols, 20);
    }

    #[test]
    fn an_escape_inside_a_sequence_starts_the_next_one() {
        let vt = vt(b"\x1b[3\x1b[2;2Hok");
        assert_eq!(vt.grid().row_text(0), "");
        assert_eq!(vt.grid().row_text(1), " ok");
    }

    #[test]
    fn an_unrecognised_sequence_is_skipped_rather_than_printed() {
        let vt = vt(b"\x1b]0;a window title\x07\x1b[?25lok");
        assert_eq!(vt.grid().row_text(0), "ok");
    }
}
