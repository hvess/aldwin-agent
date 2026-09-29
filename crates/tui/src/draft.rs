//! The multi-line draft and its layout as visual rows.
//!
//! [`Layout`] is the only source-to-screen mapping: `ui::chrome` draws from
//! it, `App::handle_key` navigates by it, `ui::draw` sizes the composer
//! from it. Never wrap the draft with `Paragraph`; it does not report its
//! breaks, so the caret would drift from the text.
//!
//! Positions are character indices, matching [`Draft::cursor`]; columns are
//! display cells.

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;

use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use unicode_width::UnicodeWidthChar;

/// One visual row: the half-open character range of the draft it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    pub start: usize,
    pub end: usize,
}

/// A draft wrapped to a column width.
#[derive(Debug)]
pub(crate) struct Layout {
    chars: Vec<char>,
    rows: Vec<Row>,
}

impl Layout {
    /// Wraps `text` to `width` cells, at the row's last space or else
    /// mid-word; `\n` always ends a row. Never zero rows: an empty draft is
    /// one empty row.
    pub(crate) fn new(text: &str, width: usize) -> Self {
        let width = width.max(1);
        let chars: Vec<char> = text.chars().collect();
        let mut rows: Vec<Row> = Vec::new();
        let mut start = 0usize;
        let mut col = 0usize;
        // Just past the row's last space: a soft break keeps the space on
        // the row it ends.
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
                // Re-test this character on the new row: carried-over text
                // can still leave it too wide.
                continue;
            }
            col += w;
            i += 1;
            if ch == ' ' {
                last_space = Some(i);
            }
        }
        rows.push(Row {
            start,
            end: chars.len(),
        });
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
    /// On a soft break the cursor belongs to the row it opens, where the
    /// next character lands; on a `\n` it stays at the end of the row it
    /// closes.
    pub(crate) fn position(&self, cursor: usize) -> (usize, usize) {
        let cursor = cursor.min(self.chars.len());
        // Rows are in order: skip, by bisection, every row ending before it.
        let from = self.rows.partition_point(|row| row.end < cursor);
        for (i, row) in self.rows.iter().enumerate().skip(from) {
            let last = i + 1 == self.rows.len();
            let hard = !last && self.rows[i + 1].start > row.end;
            if cursor < row.end || (cursor == row.end && (last || hard)) {
                return (i, self.width_of(row.start, cursor));
            }
        }
        // Unreachable (`rows` is never empty); clamped, not a panic, on a
        // draw path.
        (self.rows.len().saturating_sub(1), 0)
    }

    /// The character index `delta` rows from `cursor`, holding the column
    /// where the row allows. `None` past the first or last row;
    /// `App::handle_key` then scrolls the log instead.
    pub(crate) fn step_row(&self, cursor: usize, delta: isize) -> Option<usize> {
        let (row, col) = self.position(cursor);
        let target = row
            .checked_add_signed(delta)
            .filter(|&t| t < self.rows.len())?;
        Some(self.index_at(target, col))
    }

    /// The character index at column `col` of row `i`, clamped to the row.
    ///
    /// A soft-wrapped row clamps one short of its end: its end index is the
    /// next row's start, which [`Self::position`] puts on the next row.
    fn index_at(&self, i: usize, col: usize) -> usize {
        let Some(row) = self.rows.get(i) else {
            return self.chars.len();
        };
        let soft = self
            .rows
            .get(i + 1)
            .is_some_and(|next| next.start == row.end);
        let limit = if soft {
            row.end.saturating_sub(1).max(row.start)
        } else {
            row.end
        };
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
        self.chars[from..to.max(from)]
            .iter()
            .map(|c| c.width().unwrap_or(0))
            .sum()
    }
}

/// The half-open character range of the source line `cursor` is on; Home
/// and End move within it.
///
/// Walks rather than collecting to `Vec<char>`: it runs per keystroke on
/// possibly large pastes.
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
            // `None`: a cursor past the end clamps to it.
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

/// Makes pasted text safe to hold in a draft: `\r\n` and `\r` become `\n`;
/// a tab becomes [`TAB`], since the terminal's tab stops would put the
/// caret off the text; every other control character, `ESC` included, is
/// dropped so a draft cannot drive the terminal. Borrowed when unchanged.
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

/// Exactly the characters [`sanitize`] rewrites; its fast path and rewrite
/// both use this, so they cannot disagree.
fn needs_rewriting(c: char) -> bool {
    c.is_control() && c != '\n'
}

/// What a tab becomes in a paste and wherever one would be drawn (a fence,
/// a diff).
pub(crate) const TAB: &str = "    ";

/// `text` with each tab as [`TAB`]; a raw tab has no cell and would drop a
/// line's indentation.
pub(crate) fn expand_tabs(text: &str) -> String {
    text.replace('\t', TAB)
}

/// Text being typed and its caret, a character index; the composer and a
/// review comment both use it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Draft {
    text: String,
    cursor: usize,
    layout: LayoutCache,
}

/// The last [`Layout`] built, with its width: the composer is measured every
/// frame and a paste can be long. Every change to the text must drop it
/// ([`Draft::edited`]).
#[derive(Debug, Default)]
struct LayoutCache(RefCell<Option<(usize, Rc<Layout>)>>);

/// A cache, not state: a copy starts empty, and it never makes two drafts
/// differ.
impl Clone for LayoutCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for LayoutCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for LayoutCache {}

impl Draft {
    /// The text wrapped to `width`, built once per edit and width.
    pub(crate) fn layout(&self, width: usize) -> Rc<Layout> {
        let mut cached = self.layout.0.borrow_mut();
        match &*cached {
            Some((at, layout)) if *at == width => Rc::clone(layout),
            _ => {
                let layout = Rc::new(Layout::new(&self.text, width));
                *cached = Some((width, Rc::clone(&layout)));
                layout
            }
        }
    }

    fn edited(&mut self) {
        *self.layout.0.get_mut() = None;
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replaces the text, the caret at its end.
    pub(crate) fn set(&mut self, text: String) {
        self.cursor = text.chars().count();
        self.text = text;
        self.edited();
    }

    /// Empties the draft and hands back what it held.
    pub(crate) fn take(&mut self) -> String {
        self.cursor = 0;
        self.edited();
        std::mem::take(&mut self.text)
    }

    pub(crate) fn move_to(&mut self, cursor: usize) {
        self.cursor = cursor.min(self.text.chars().count());
    }

    pub(crate) fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
        self.edited();
    }

    pub(crate) fn insert_str(&mut self, text: &str) {
        let at = self.byte_at(self.cursor);
        self.text.insert_str(at, text);
        self.cursor += text.chars().count();
        self.edited();
    }

    /// Applies an editing key (characters without Ctrl, `⌫`, `⌦`, `←` `→`,
    /// Home, End); `false` leaves any other key to the caller.
    pub(crate) fn edit(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        match code {
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => self.insert(c),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let at = self.byte_at(self.cursor);
                    self.text.remove(at);
                    self.edited();
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.text.chars().count() {
                    let at = self.byte_at(self.cursor);
                    self.text.remove(at);
                    self.edited();
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.move_to(self.cursor + 1),
            KeyCode::Home => self.cursor = source_line(&self.text, self.cursor).0,
            KeyCode::End => self.cursor = source_line(&self.text, self.cursor).1,
            _ => return false,
        }
        true
    }

    fn byte_at(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, width: usize) -> Vec<String> {
        let layout = Layout::new(text, width);
        (0..layout.row_count())
            .map(|i| layout.row_text(i))
            .collect()
    }

    #[test]
    fn an_empty_draft_is_one_empty_row() {
        assert_eq!(rows("", 10), vec![""]);
    }

    #[test]
    fn a_layout_is_built_again_after_every_edit_and_only_then() {
        let mut draft = Draft::default();
        draft.set("one two".into());
        let first = draft.layout(6);
        assert!(
            Rc::ptr_eq(&first, &draft.layout(6)),
            "unchanged: the same layout"
        );
        assert!(
            !Rc::ptr_eq(&first, &draft.layout(7)),
            "another width: built again"
        );
        draft.insert_str(" three");
        assert_eq!(draft.layout(6).row_text(2), "three");
        draft.edit(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!(draft.layout(6).row_text(2), "thre");
        draft.take();
        assert_eq!(draft.layout(6).row_count(), 1);
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
            let w: usize = layout
                .row_text(i)
                .chars()
                .map(|c| c.width().unwrap_or(0))
                .sum();
            assert!(
                w <= 7,
                "row {i} is {w} cells wide: {:?}",
                layout.row_text(i)
            );
        }
    }

    #[test]
    fn the_cursor_is_reported_on_the_wrapped_row_not_the_source_line() {
        let layout = Layout::new("hello there world", 11);
        assert_eq!(layout.position(0), (0, 0));
        assert_eq!(
            layout.position(6),
            (1, 0),
            "the cursor after the break opens the second row"
        );
        assert_eq!(
            layout.position(17),
            (1, 11),
            "and the end of the draft is at the end of the last row"
        );
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
        assert_eq!(
            layout.step_row(5, 1),
            Some(9),
            "column 5 clamps to the end of the two-character row"
        );
        assert_eq!(
            layout.step_row(0, -1),
            None,
            "there is no row above the first"
        );
        assert_eq!(layout.step_row(13, 1), None, "and none below the last");
    }

    #[test]
    fn stepping_moves_by_wrapped_row_not_by_source_line() {
        // Down must reach the second row, not scroll the log.
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
        assert_eq!(
            sanitize("a\x1b[31mb\x07"),
            "a[31mb",
            "escapes lose their control characters, not their text"
        );
    }

    /// Also pins the fast path to reject exactly what the rewrite changes.
    #[test]
    fn a_paste_that_needs_no_rewriting_is_not_copied() {
        assert!(matches!(
            sanitize("plain text\nover two lines"),
            Cow::Borrowed(_)
        ));
        for altered in ["a\rb", "a\tb", "a\u{7}b", "a\u{1b}b", "a\u{85}b"] {
            assert!(
                matches!(sanitize(altered), Cow::Owned(_)),
                "{altered:?} needs rewriting and must not be borrowed back"
            );
            assert_ne!(
                sanitize(altered),
                altered,
                "and the rewrite must actually change it"
            );
        }
    }

    #[test]
    fn a_draft_edits_by_character() {
        let mut d = Draft::default();
        for c in "héllo".chars() {
            assert!(d.edit(KeyCode::Char(c), KeyModifiers::NONE));
        }
        d.edit(KeyCode::Left, KeyModifiers::NONE);
        d.edit(KeyCode::Left, KeyModifiers::NONE);
        d.edit(KeyCode::Backspace, KeyModifiers::NONE);
        assert_eq!((d.text(), d.cursor()), ("hélo", 2));
        d.edit(KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(d.text(), "héo");
        d.edit(KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(d.cursor(), 0);
        d.edit(KeyCode::End, KeyModifiers::NONE);
        assert_eq!(d.cursor(), 3);
        assert!(
            !d.edit(KeyCode::Char('c'), KeyModifiers::CONTROL),
            "a control chord is the caller's"
        );
        assert!(!d.edit(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(d.take(), "héo");
        assert!(d.is_empty() && d.cursor() == 0);
    }
}
