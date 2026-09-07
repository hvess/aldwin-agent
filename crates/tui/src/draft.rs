//! The composer's draft, laid out as visual rows.
//!
//! The composer is a real multi-line editor, not a one-line field: Shift+
//! Enter (and Ctrl+J) open a new line, and a bracketed paste drops an
//! arbitrary block of text in whole. Both mean the draft's *source* lines
//! and the rows it occupies on screen stop being the same thing — a pasted
//! 200-column line is one source line and three screen rows, and a pasted
//! 40-line file is more rows than the composer is ever allowed to take.
//!
//! [`Layout`] is the one place that mapping lives. `ui::chrome` draws from
//! it, `App::handle_key` navigates by it, and `ui::draw` sizes the composer
//! band from it — so the caret cannot land somewhere the text isn't, the
//! way it did while the draft was wrapped by `Paragraph` (which reports
//! nothing about where it broke) and the cursor was placed from the
//! *source* line and column.
//!
//! Positions are **character** indices into the draft, matching
//! `App::cursor`; columns are **display cells**, matching the screen.

use std::borrow::Cow;

use unicode_width::UnicodeWidthChar;

/// One visual row: the half-open character range of the draft it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    pub start: usize,
    pub end:   usize,
}

/// A draft wrapped to a column width.
pub(crate) struct Layout {
    chars: Vec<char>,
    rows:  Vec<Row>,
}

impl Layout {
    /// Wraps `text` to `width` display cells, breaking at the last space on
    /// the row where there is one and mid-word where there isn't. A `\n`
    /// always ends a row, and an empty draft is one empty row — the
    /// composer is never zero rows tall.
    pub(crate) fn new(text: &str, width: usize) -> Self {
        let width = width.max(1);
        let chars: Vec<char> = text.chars().collect();
        let mut rows: Vec<Row> = Vec::new();
        let mut start = 0usize;
        let mut col = 0usize;
        // Just past the last space on the current row — where a soft break
        // goes, so the space stays with the row it ended rather than
        // opening the next one.
        let mut last_space: Option<usize> = None;
        let mut i = 0usize;
        while i < chars.len() {
            let ch = chars[i];
            if ch == '\n' {
                rows.push(Row { start, end: i });
                i += 1;
                start = i;
                col = 0;
                last_space = None;
                continue;
            }
            let w = ch.width().unwrap_or(0);
            if col + w > width && i > start {
                let brk = last_space.filter(|&b| b > start && b <= i).unwrap_or(i);
                rows.push(Row { start, end: brk });
                start = brk;
                col = chars[start..i].iter().map(|c| c.width().unwrap_or(0)).sum();
                last_space = None;
                // Re-test this character against the row it just opened
                // rather than assuming it fits — a break that carried text
                // over can still leave it too wide.
                continue;
            }
            col += w;
            i += 1;
            if ch == ' ' {
                last_space = Some(i);
            }
        }
        rows.push(Row { start, end: chars.len() });
        Self { chars, rows }
    }

    pub(crate) fn row_count(&self) -> usize {
        self.rows.len()
    }

    /// Row `i`'s text, or the empty string past the end.
    pub(crate) fn row_text(&self, i: usize) -> String {
        match self.rows.get(i) {
            Some(row) => self.chars[row.start..row.end].iter().collect(),
            None => String::new(),
        }
    }

    /// Where `cursor` sits: `(row, display column)`.
    ///
    /// A cursor exactly on a *soft* break belongs to the row it opens, not
    /// the one it closed — that is the cell the next typed character will
    /// occupy. On a *hard* break (a `\n`) it stays on the row it ends,
    /// which is the end of that line.
    pub(crate) fn position(&self, cursor: usize) -> (usize, usize) {
        let cursor = cursor.min(self.chars.len());
        for (i, row) in self.rows.iter().enumerate() {
            let last = i + 1 == self.rows.len();
            let hard = !last && self.rows[i + 1].start > row.end;
            if cursor < row.end || (cursor == row.end && (last || hard)) {
                return (i, self.width_of(row.start, cursor));
            }
        }
        // Unreachable while `rows` is non-empty (it always is), but a
        // clamped answer beats a panic in a draw path.
        (self.rows.len().saturating_sub(1), 0)
    }

    /// The character index `delta` rows away from `cursor`, holding the
    /// display column where the target row is wide enough. `None` when
    /// there is no such row — which is how `App::handle_key` knows an
    /// Up/Down press has run out of draft and should scroll the log
    /// instead.
    pub(crate) fn step_row(&self, cursor: usize, delta: isize) -> Option<usize> {
        let (row, col) = self.position(cursor);
        let target = row.checked_add_signed(delta).filter(|&t| t < self.rows.len())?;
        Some(self.index_at(target, col))
    }

    /// The character index at display column `col` of row `i`, clamped to
    /// the last position that still reads as being *on* that row.
    ///
    /// On a soft-wrapped row that is one short of its end: the index at the
    /// end of such a row is the same index as the start of the next one,
    /// and [`Self::position`] resolves it to the next row (which is where
    /// the caret visibly is). Clamping to the end would make Up from a full
    /// row appear not to move at all.
    fn index_at(&self, i: usize, col: usize) -> usize {
        let Some(row) = self.rows.get(i) else { return self.chars.len() };
        let soft = self.rows.get(i + 1).is_some_and(|next| next.start == row.end);
        let limit = if soft { row.end.saturating_sub(1).max(row.start) } else { row.end };
        let mut used = 0usize;
        for idx in row.start..limit {
            let w = self.chars[idx].width().unwrap_or(0);
            if used + w > col {
                return idx;
            }
            used += w;
        }
        limit
    }

    fn width_of(&self, from: usize, to: usize) -> usize {
        self.chars[from..to.max(from)].iter().map(|c| c.width().unwrap_or(0)).sum()
    }
}

/// The half-open character range of the source line `cursor` is on — what
/// Home and End move between, so both stay inside the line the caret is on
/// rather than jumping to the ends of a whole pasted block.
///
/// Walks the draft rather than collecting it: this runs on a keystroke, and
/// a `Vec<char>` of a pasted file is four bytes per character of it for two
/// answers that a single pass already has.
pub(crate) fn source_line(text: &str, cursor: usize) -> (usize, usize) {
    let mut chars = text.chars();
    let mut start = 0usize;
    let mut i = 0usize;
    while i < cursor {
        match chars.next() {
            Some('\n') => {
                i += 1;
                start = i;
            }
            // A cursor past the end of the draft clamps to it, rather than
            // reporting a line that isn't there.
            Some(_) => i += 1,
            None => break,
        }
    }
    let mut end = i;
    for c in chars {
        if c == '\n' {
            break;
        }
        end += 1;
    }
    (start, end)
}

/// Makes pasted text safe to hold in a draft.
///
/// A paste arrives as whatever was on the clipboard, and three kinds of
/// character in it would otherwise break the composer rather than land in
/// it:
///
/// * `\r\n` and a lone `\r` are line breaks from another platform's
///   convention — normalized, or a Windows-clipboard paste ends every row
///   with a stray control character.
/// * a tab advances the *terminal's* cursor to its own next tab stop,
///   which no cell-grid layout here can predict; it becomes four spaces, so
///   the caret and the text agree about where they are. This is the one
///   place the harness alters what the developer pasted, and it is
///   deliberate — the alternative is a caret that drifts further from the
///   text with every tab on the row.
/// * every other control character (an ANSI escape's own `ESC` included) is
///   dropped outright: nothing in a draft should be able to move the
///   terminal's cursor or change its modes.
///
/// Borrowed back unchanged when none of that applies, which is the ordinary
/// case — a paste is a whole clipboard, and rewriting one that needed no
/// rewriting copies all of it to produce the same bytes.
pub(crate) fn sanitize(text: &str) -> Cow<'_, str> {
    if !text.chars().any(needs_rewriting) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\t' => out.push_str(TAB),
            c if needs_rewriting(c) => {}
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Exactly the characters [`sanitize`] would not pass through untouched —
/// one statement of the rule, so the borrowed fast path above cannot come
/// to disagree with the rewrite below it.
fn needs_rewriting(c: char) -> bool {
    c.is_control() && c != '\n'
}

/// What a tab becomes. Four, matching the code most of the transcript
/// renders.
const TAB: &str = "    ";

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, width: usize) -> Vec<String> {
        let layout = Layout::new(text, width);
        (0..layout.row_count()).map(|i| layout.row_text(i)).collect()
    }

    #[test]
    fn an_empty_draft_is_one_empty_row() {
        assert_eq!(rows("", 10), vec![""]);
    }

    #[test]
    fn newlines_break_rows_and_an_empty_line_keeps_its_own() {
        assert_eq!(rows("a\n\nb", 10), vec!["a", "", "b"]);
    }

    #[test]
    fn a_long_line_wraps_at_the_last_space() {
        assert_eq!(rows("hello there world", 11), vec!["hello ", "there world"]);
    }

    #[test]
    fn a_word_longer_than_the_column_breaks_mid_word_rather_than_overflowing() {
        assert_eq!(rows("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn every_row_fits_the_column_even_with_wide_characters() {
        let layout = Layout::new("日本語のテキストです", 7);
        for i in 0..layout.row_count() {
            let w: usize = layout.row_text(i).chars().map(|c| c.width().unwrap_or(0)).sum();
            assert!(w <= 7, "row {i} is {w} cells wide: {:?}", layout.row_text(i));
        }
    }

    /// The whole point of the type: the caret has to land where the text
    /// actually is, on the row the wrapper actually broke.
    #[test]
    fn the_cursor_is_reported_on_the_wrapped_row_not_the_source_line() {
        let layout = Layout::new("hello there world", 11);
        assert_eq!(layout.position(0), (0, 0));
        assert_eq!(layout.position(6), (1, 0), "the cursor after the break opens the second row");
        assert_eq!(layout.position(17), (1, 11), "and the end of the draft is at the end of the last row");
    }

    #[test]
    fn a_cursor_at_a_hard_break_stays_at_the_end_of_the_line_it_closes() {
        let layout = Layout::new("ab\ncd", 10);
        assert_eq!(layout.position(2), (0, 2));
        assert_eq!(layout.position(3), (1, 0));
    }

    #[test]
    fn stepping_a_row_holds_the_column_and_clamps_to_a_shorter_row() {
        let layout = Layout::new("abcdef\nxy\nlonger", 20);
        assert_eq!(layout.step_row(5, 1), Some(9), "column 5 clamps to the end of the two-character row");
        assert_eq!(layout.step_row(0, -1), None, "there is no row above the first");
        assert_eq!(layout.step_row(13, 1), None, "and none below the last");
    }

    #[test]
    fn stepping_moves_by_wrapped_row_not_by_source_line() {
        // One source line, three rows: Down from the first must reach the
        // second row, not fall through to the log.
        let layout = Layout::new("aaaa bbbb cccc", 5);
        assert_eq!(layout.row_count(), 3);
        assert!(layout.step_row(0, 1).is_some());
    }

    #[test]
    fn home_and_end_scope_to_the_line_the_cursor_is_on() {
        assert_eq!(source_line("one\ntwo\nthree", 5), (4, 7));
        assert_eq!(source_line("one\ntwo\nthree", 0), (0, 3));
    }

    #[test]
    fn a_paste_is_normalized_rather_than_taken_literally() {
        assert_eq!(sanitize("a\r\nb\rc"), "a\nb\nc");
        assert_eq!(sanitize("a\tb"), "a    b");
        assert_eq!(sanitize("a\x1b[31mb\x07"), "a[31mb", "escapes lose their control characters, not their text");
    }

    /// The ordinary paste — nothing to rewrite — must come back borrowed,
    /// or every paste copies a whole clipboard to produce the same bytes.
    /// Also pins the two halves of the rule against each other: the fast
    /// path is only correct while it rejects exactly what the rewrite would
    /// have touched.
    #[test]
    fn a_paste_that_needs_no_rewriting_is_not_copied() {
        assert!(matches!(sanitize("plain text\nover two lines"), Cow::Borrowed(_)));
        for altered in ["a\rb", "a\tb", "a\u{7}b", "a\u{1b}b", "a\u{85}b"] {
            assert!(matches!(sanitize(altered), Cow::Owned(_)), "{altered:?} needs rewriting and must not be borrowed back");
            assert_ne!(sanitize(altered), altered, "and the rewrite must actually change it");
        }
    }
}
