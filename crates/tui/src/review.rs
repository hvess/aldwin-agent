//! The review's state (ADR 0009 §4): the files of a staged changeset as the
//! design draws them — a tree with reading progress, a diff with folded
//! runs, a selection made with the mouse (ADR 0010), and the comments left
//! on it — and what each key and click does to that.
//!
//! Pure state, no drawing: `ui::review` reads this and `App` drives it, so
//! every rule here is testable without a terminal.

use aldwin_core::{Changeset, ReviewComment, ReviewDecision};
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};

use crate::scroll::WHEEL_ROWS;

/// Rows `PgUp` and `PgDn` scroll the diff.
const PAGE_ROWS: usize = 10;

/// Where the diff's rows were drawn last frame: the screen rect of the rows
/// themselves (not the pane's header) and the index of the drawn row at its
/// top. Only valid for the rows it was drawn from — anything that changes
/// them (another file, an opened fold) drops it until the next draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pane {
    pub x:      u16,
    pub y:      u16,
    pub width:  u16,
    pub height: u16,
    pub top:    usize,
}

/// Unchanged lines kept on each side of a change; the rest fold. One, as
/// the frame draws it: `⋯ 141 lines`, line 143, the change, line 150, `⋯ 8
/// lines`.
const CONTEXT: usize = 1;

/// One row of a file's diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    /// A folded run of unchanged lines. `first` is the row index into the
    /// unfolded list where it starts; `len` how many it hides.
    Fold { first: usize, len: usize },
    /// Unchanged, with its line number in the file as it would be written.
    Context { line: usize, text: String },
    Add { line: usize, text: String },
    /// Removed; no line in the new file, so it carries the number of the
    /// nearest line after it for a comment to anchor on.
    Del { after: usize, text: String },
}

impl DiffRow {
    /// The new-file line a comment on this row anchors to.
    pub fn anchor(&self) -> Option<usize> {
        match self {
            DiffRow::Context { line, .. } | DiffRow::Add { line, .. } => Some(*line),
            DiffRow::Del { after, .. } => Some(*after),
            DiffRow::Fold { .. } => None,
        }
    }
}

/// A comment left on a run of lines, not yet sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingComment {
    /// Inclusive new-file line numbers.
    pub lines: (usize, usize),
    pub text:  String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFile {
    pub path:     String,
    /// The file did not exist before — the tree draws ` +` beside it.
    pub added:    bool,
    /// Every row, unfolded.
    unfolded:     Vec<DiffRow>,
    /// Folds over `unfolded`, as (start row, length), in order. Expanded
    /// ones are removed from this list.
    folds:        Vec<(usize, usize)>,
    /// `+11 −2`.
    pub added_lines:   usize,
    pub removed_lines: usize,
    /// The developer has seen the whole of it — `⌃↩` waits for every file.
    pub read:     bool,
    pub comments: Vec<PendingComment>,
}

impl ReviewFile {
    /// Each drawn row as the run of unfolded rows it stands for: `(start,
    /// len, folded)`. A fold is every line it hides; any other row is one.
    fn spans(&self) -> impl Iterator<Item = (usize, usize, bool)> + '_ {
        let mut i = 0;
        let mut folds = self.folds.iter().peekable();
        std::iter::from_fn(move || {
            if i >= self.unfolded.len() {
                return None;
            }
            let span = match folds.peek() {
                Some(&&(start, len)) if start == i => {
                    folds.next();
                    (start, len, true)
                }
                _ => (i, 1, false),
            };
            i += span.1;
            Some(span)
        })
    }

    /// The rows as drawn: folds collapsed to one row each.
    pub fn rows(&self) -> Vec<DiffRow> {
        self.spans()
            .map(|(start, len, folded)| if folded { DiffRow::Fold { first: start, len } } else { self.unfolded[start].clone() })
            .collect()
    }

    /// The unfolded rows drawn row `row` stands for, first and last.
    fn span_of(&self, row: usize) -> Option<(usize, usize)> {
        self.spans().nth(row).map(|(start, len, _)| (start, start + len - 1))
    }

    /// The drawn row that shows unfolded row `unfolded` — its own, or the
    /// fold hiding it.
    fn row_of(&self, unfolded: usize) -> usize {
        self.spans().position(|(start, len, _)| unfolded < start + len).unwrap_or(0)
    }

    /// Opens the fold starting at unfolded row `first`.
    pub fn expand(&mut self, first: usize) {
        self.folds.retain(|&(start, _)| start != first);
    }

    /// Opens every fold — the keyboard's way to the whole file.
    pub(crate) fn expand_all(&mut self) {
        self.folds.clear();
    }

    pub(crate) fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }
}

/// A selection, kept in *unfolded* rows so that opening a fold — which
/// renumbers every drawn row after it — cannot move it onto other lines.
/// Each end is the span of the drawn row it was made on: a drag that ends
/// on a fold takes every line the fold hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    /// Where the press landed.
    anchor: (usize, usize),
    /// Where the drag is now.
    head:   (usize, usize),
}

impl Selection {
    /// The unfolded rows covered, first and last.
    fn lines(self) -> (usize, usize) {
        (self.anchor.0.min(self.head.0), self.anchor.1.max(self.head.1))
    }
}

/// The comment being typed for a selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommentDraft {
    pub text:   String,
    pub cursor: usize,
}

/// What one key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewOutcome {
    Stay,
    /// The developer decided; the caller sends it.
    Decide(ReviewDecision),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub review_id: String,
    pub files:     Vec<ReviewFile>,
    pub current:   usize,
    /// The lines selected for a comment in the current file. There is no
    /// line cursor — lines are selected with the mouse (ADR 0010), so a row
    /// is marked only when it is part of a selection.
    selected:      Option<Selection>,
    /// A button is down on the diff and the selection follows the pointer.
    dragging:      bool,
    /// First drawn row of the current file's pane. The keys and the wheel
    /// move it, clamped to the last full pane once one has been drawn; the
    /// drawing side clamps it again (the pane can shrink) and writes it back.
    pub scroll:    usize,
    /// Where the drawing side put the diff's rows on screen last frame —
    /// what a click is measured against.
    pub(crate) pane: Option<Pane>,
    pub comment:   Option<CommentDraft>,
    /// `⎋` with nothing selected asks before dropping the changes.
    pub confirm_discard: bool,
    /// `?` toggles the key list in the footer.
    pub keys_shown: bool,
}

impl Review {
    pub fn open(review_id: String, changeset: Changeset) -> Self {
        let files = changeset.files.into_iter().map(|f| file_from(&f.path, f.before.as_deref(), &f.after)).collect();
        Self {
            review_id,
            files,
            current: 0,
            selected: None,
            dragging: false,
            scroll: 0,
            pane: None,
            comment: None,
            confirm_discard: false,
            keys_shown: false,
        }
    }

    pub fn file(&self) -> &ReviewFile {
        &self.files[self.current.min(self.files.len().saturating_sub(1))]
    }

    fn file_mut(&mut self) -> &mut ReviewFile {
        let i = self.current.min(self.files.len().saturating_sub(1));
        &mut self.files[i]
    }

    pub fn files_read(&self) -> usize {
        self.files.iter().filter(|f| f.read).count()
    }

    pub fn all_read(&self) -> bool {
        self.files.iter().all(|f| f.read)
    }

    pub fn comment_count(&self) -> usize {
        self.files.iter().map(|f| f.comments.len()).sum()
    }

    /// The drawn rows selected, as an inclusive index range into the
    /// current file's rows.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let (first, last) = self.selected?.lines();
        Some((self.file().row_of(first), self.file().row_of(last)))
    }

    /// Selects drawn rows `from` through `to` of the current file — what a
    /// press on one and a drag to the other leaves behind.
    pub fn select(&mut self, from: usize, to: usize) {
        let file = self.file();
        if let (Some(anchor), Some(head)) = (file.span_of(from), file.span_of(to)) {
            self.selected = Some(Selection { anchor, head });
        }
    }

    /// An arrow with a selection: `extend` (Shift) carries its moving end a
    /// row up or down; without it the selection becomes the one line past
    /// that end. With nothing selected yet, Shift selects the first line
    /// shown — there is no cursor to start from, so the top of the pane is
    /// where the eye already is.
    fn step_selection(&mut self, up: bool, extend: bool) {
        let rows = self.file().rows().len();
        let Some(selected) = self.selected else {
            let top = self.scroll.min(rows.saturating_sub(1));
            self.select(top, top);
            return;
        };
        let head = self.file().row_of(selected.head.0);
        let next = if up { head.saturating_sub(1) } else { (head + 1).min(rows.saturating_sub(1)) };
        let Some(span) = self.file().span_of(next) else { return };
        self.selected = Some(if extend { Selection { head: span, ..selected } } else { Selection { anchor: span, head: span } });
        self.keep_in_view(next);
    }

    /// Scrolls just far enough that drawn row `row` is in the pane.
    fn keep_in_view(&mut self, row: usize) {
        let height = self.pane.map_or(1, |p| p.height.max(1) as usize);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
    }

    /// Scrolls the diff by `delta` rows, never past the last full pane.
    /// Before the first draw there is no pane to measure, so the drawing
    /// side's clamp is the only one.
    fn scroll_by(&mut self, delta: isize) {
        let rows = self.file().rows().len();
        let last_top = self.pane.map_or(usize::MAX, |p| rows.saturating_sub(p.height as usize));
        self.scroll = self.scroll.saturating_add_signed(delta).min(last_top);
    }

    /// Something changed which rows are drawn: the pane recorded last frame
    /// no longer says what is under a click, so none is honoured until the
    /// next draw records it again.
    fn rows_changed(&mut self) {
        self.pane = None;
    }

    /// One mouse event over the review. Only the diff pane answers: a press
    /// on a line selects it and starts a drag, a drag carries the selection
    /// to the row under the pointer, a release ends it, and a press on a
    /// fold opens it. The wheel scrolls the diff wherever it is.
    pub fn handle_mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        if self.comment.is_some() || self.confirm_discard {
            return;
        }
        match kind {
            MouseEventKind::ScrollUp => self.scroll_by(-(WHEEL_ROWS as isize)),
            MouseEventKind::ScrollDown => self.scroll_by(WHEEL_ROWS as isize),
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(at) = self.row_at(column, row, false) else { return };
                if let Some(DiffRow::Fold { first, .. }) = self.file().rows().get(at) {
                    let first = *first;
                    self.file_mut().expand(first);
                    self.rows_changed();
                    return;
                }
                self.select(at, at);
                self.dragging = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                // Past the pane's top or bottom edge the selection stops at
                // the last row shown rather than being dropped.
                let head = self.row_at(column, row, true).and_then(|at| self.file().span_of(at));
                if let (Some(head), Some(selected)) = (head, &mut self.selected) {
                    selected.head = head;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            _ => {}
        }
    }

    /// The row index under a screen cell, or `None` off the diff. With
    /// `clamp`, a point above or below the pane (a drag carried past its
    /// edge) lands on the first or last row shown.
    fn row_at(&self, column: u16, row: u16, clamp: bool) -> Option<usize> {
        let pane = self.pane?;
        let shown = self.file().rows().len().saturating_sub(pane.top).min(pane.height as usize);
        let inside = (pane.x..pane.x + pane.width).contains(&column) && (pane.y..pane.y + shown as u16).contains(&row);
        if shown == 0 || !(inside || clamp) {
            return None;
        }
        Some(pane.top + (row.saturating_sub(pane.y) as usize).min(shown - 1))
    }

    /// `router.rs · 144–145` and `2 lines` — what the comment field's label
    /// names.
    pub fn selection_label(&self) -> Option<(String, String)> {
        let (first, last) = self.selected?.lines();
        let lines = self.line_range(first, last)?;
        let name = self.file().path.rsplit('/').next().unwrap_or(&self.file().path).to_string();
        let where_ = if lines.0 == lines.1 { format!("{name} · {}", lines.0) } else { format!("{name} · {}–{}", lines.0, lines.1) };
        let n = last - first + 1;
        Some((format!("{n} {}", if n == 1 { "line" } else { "lines" }), where_))
    }

    /// The new-file lines unfolded rows `first` through `last` anchor to.
    fn line_range(&self, first: usize, last: usize) -> Option<(usize, usize)> {
        let anchors: Vec<usize> = self.file().unfolded.get(first..=last)?.iter().filter_map(DiffRow::anchor).collect();
        Some((*anchors.iter().min()?, *anchors.iter().max()?))
    }

    /// The drawing side says the bottom of the current file was on screen.
    pub fn mark_read(&mut self) {
        self.file_mut().read = true;
    }

    /// Every pending comment, plus `general` if it is not empty, as the
    /// decision to send.
    fn comments_decision(&self, general: &str) -> Option<ReviewDecision> {
        let mut comments: Vec<ReviewComment> = self
            .files
            .iter()
            .flat_map(|f| f.comments.iter().map(move |c| ReviewComment { path: f.path.clone(), lines: c.lines, text: c.text.clone() }))
            .collect();
        if !general.trim().is_empty() {
            comments.push(ReviewComment { path: String::new(), lines: (0, 0), text: general.trim().to_string() });
        }
        (!comments.is_empty()).then_some(ReviewDecision::Comment { comments })
    }

    /// One key, with the review's own field text (`general`, what the
    /// developer typed into "Ask for a change") for the keys that send.
    /// Typing into that field is the caller's; this handles the rest.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers, general: &str) -> ReviewOutcome {
        if self.confirm_discard {
            return match code {
                KeyCode::Char('1') | KeyCode::Esc => {
                    self.confirm_discard = false;
                    ReviewOutcome::Stay
                }
                KeyCode::Char('2') => ReviewOutcome::Decide(ReviewDecision::Discard),
                _ => ReviewOutcome::Stay,
            };
        }
        if let Some(draft) = &mut self.comment {
            match code {
                KeyCode::Esc => self.comment = None,
                KeyCode::Enter => {
                    let text = draft.text.trim().to_string();
                    if !text.is_empty() {
                        let lines = self.selected.and_then(|s| self.line_range(s.lines().0, s.lines().1));
                        if let Some(lines) = lines {
                            self.file_mut().comments.push(PendingComment { lines, text });
                        }
                    }
                    self.comment = None;
                    self.selected = None;
                }
                KeyCode::Backspace => {
                    if draft.cursor > 0 {
                        draft.cursor -= 1;
                        let at = draft.text.char_indices().nth(draft.cursor).map_or(draft.text.len(), |(i, _)| i);
                        draft.text.remove(at);
                    }
                }
                KeyCode::Left => draft.cursor = draft.cursor.saturating_sub(1),
                KeyCode::Right => draft.cursor = (draft.cursor + 1).min(draft.text.chars().count()),
                KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                    let at = draft.text.char_indices().nth(draft.cursor).map_or(draft.text.len(), |(i, _)| i);
                    draft.text.insert(at, c);
                    draft.cursor += 1;
                }
                _ => {}
            }
            return ReviewOutcome::Stay;
        }

        let shift = modifiers.contains(KeyModifiers::SHIFT);
        match code {
            // The keyboard's way to a selection (HIG, "Keyboards": Shift and
            // an arrow extends a selection). With nothing selected the
            // arrows scroll; once a line is selected they move it.
            KeyCode::Up | KeyCode::Down if shift || self.selected.is_some() => self.step_selection(code == KeyCode::Up, shift),
            KeyCode::Up => self.scroll_by(-1),
            KeyCode::Down => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-(PAGE_ROWS as isize)),
            KeyCode::PageDown => self.scroll_by(PAGE_ROWS as isize),
            // Folds open with a click; this is the keyboard's way to the
            // same lines, and the only one where the mouse is not captured.
            KeyCode::Char(' ') if general.is_empty() && self.file().has_folds() => {
                self.file_mut().expand_all();
                self.rows_changed();
            }
            KeyCode::Tab | KeyCode::Right => self.go_to_file((self.current + 1) % self.files.len().max(1)),
            KeyCode::BackTab | KeyCode::Left => self.go_to_file((self.current + self.files.len().max(1) - 1) % self.files.len().max(1)),
            KeyCode::Char('?') if general.is_empty() => self.keys_shown = !self.keys_shown,
            KeyCode::Enter if modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(decision) = self.comments_decision(general) {
                    return ReviewOutcome::Decide(decision);
                }
                if self.all_read() {
                    return ReviewOutcome::Decide(ReviewDecision::Approve);
                }
            }
            // `↩` opens a comment on a selection, sends what was typed, and
            // — with nothing selected and nothing typed — stands in for
            // `⌃↩`: a terminal without the Kitty keyboard protocol cannot
            // tell the two apart, and a review with no way to approve is a
            // review that cannot end.
            KeyCode::Enter => {
                if self.selected.is_some() {
                    self.comment = Some(CommentDraft::default());
                } else if let Some(decision) = self.comments_decision(general) {
                    return ReviewOutcome::Decide(decision);
                } else if self.all_read() {
                    return ReviewOutcome::Decide(ReviewDecision::Approve);
                }
            }
            KeyCode::Esc => {
                if self.selected.is_some() {
                    self.selected = None;
                    self.dragging = false;
                } else {
                    self.confirm_discard = true;
                }
            }
            _ => {}
        }
        ReviewOutcome::Stay
    }

    fn go_to_file(&mut self, i: usize) {
        if self.files.is_empty() {
            return;
        }
        self.current = i;
        self.selected = None;
        self.dragging = false;
        self.scroll = 0;
        self.rows_changed();
    }
}

/// A file's diff, folded. Common prefix and suffix are trimmed before the
/// LCS so a small change in a large file costs the change, not the file.
fn file_from(path: &str, before: Option<&str>, after: &str) -> ReviewFile {
    let before_lines: Vec<&str> = before.map(|b| b.lines().collect()).unwrap_or_default();
    let after_lines: Vec<&str> = after.lines().collect();

    let prefix = before_lines.iter().zip(&after_lines).take_while(|(a, b)| a == b).count();
    let max_suffix = before_lines.len().min(after_lines.len()) - prefix;
    let suffix = before_lines.iter().rev().zip(after_lines.iter().rev()).take(max_suffix).take_while(|(a, b)| a == b).count();

    let mut unfolded: Vec<DiffRow> = Vec::with_capacity(after_lines.len() + before_lines.len());
    for (i, text) in after_lines[..prefix].iter().enumerate() {
        unfolded.push(DiffRow::Context { line: i + 1, text: (*text).to_string() });
    }
    let mid_a = &before_lines[prefix..before_lines.len() - suffix];
    let mid_b = &after_lines[prefix..after_lines.len() - suffix];
    let mut line = prefix + 1;
    let mut added_lines = 0;
    let mut removed_lines = 0;
    let mut pending_dels: Vec<String> = Vec::new();
    for op in lcs_diff(mid_a, mid_b) {
        match op {
            Op::Same(text) => {
                for d in pending_dels.drain(..) {
                    unfolded.push(DiffRow::Del { after: line, text: d });
                }
                unfolded.push(DiffRow::Context { line, text: text.to_string() });
                line += 1;
            }
            Op::Del(text) => {
                removed_lines += 1;
                pending_dels.push(text.to_string());
            }
            Op::Add(text) => {
                for d in pending_dels.drain(..) {
                    unfolded.push(DiffRow::Del { after: line, text: d });
                }
                added_lines += 1;
                unfolded.push(DiffRow::Add { line, text: text.to_string() });
                line += 1;
            }
        }
    }
    for d in pending_dels.drain(..) {
        unfolded.push(DiffRow::Del { after: line, text: d });
    }
    for text in &after_lines[after_lines.len() - suffix..] {
        unfolded.push(DiffRow::Context { line, text: (*text).to_string() });
        line += 1;
    }

    let folds = fold_runs(&unfolded);
    ReviewFile { path: path.to_string(), added: before.is_none(), unfolded, folds, added_lines, removed_lines, read: false, comments: Vec::new() }
}

/// Runs of context longer than `2 * CONTEXT` fold, keeping `CONTEXT` lines
/// beside each change. A run at the very start or end keeps `CONTEXT` on
/// its changed side only.
fn fold_runs(rows: &[DiffRow]) -> Vec<(usize, usize)> {
    let mut folds = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        if !matches!(rows[i], DiffRow::Context { .. }) {
            i += 1;
            continue;
        }
        let start = i;
        while i < rows.len() && matches!(rows[i], DiffRow::Context { .. }) {
            i += 1;
        }
        let end = i;
        let keep_before = if start == 0 { 0 } else { CONTEXT };
        let keep_after = if end == rows.len() { 0 } else { CONTEXT };
        let fold_start = start + keep_before;
        let fold_end = end.saturating_sub(keep_after);
        if fold_end > fold_start + 1 {
            folds.push((fold_start, fold_end - fold_start));
        }
    }
    folds
}

enum Op<'a> {
    Same(&'a str),
    Del(&'a str),
    Add(&'a str),
}

fn lcs_diff<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Op<'a>> {
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op::Same(a[i]));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(Op::Del(a[i]));
            i += 1;
        } else {
            ops.push(Op::Add(b[j]));
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Del(a[i]));
        i += 1;
    }
    while j < m {
        ops.push(Op::Add(b[j]));
        j += 1;
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_core::ChangedFile;

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    fn review_of(before: Option<&str>, after: &str) -> Review {
        Review::open("r1".into(), Changeset { files: vec![ChangedFile { path: "src/x.rs".into(), before: before.map(str::to_string), after: after.into() }] })
    }

    /// The frame's own shape: a long unchanged run folds to one row with a
    /// count, one context line survives on each side of the change.
    #[test]
    fn unchanged_runs_fold_with_one_line_of_context() {
        let before = numbered(160);
        let after = before.replace("line 145\n", "line 145 changed\n");
        let r = review_of(Some(&before), &after);
        let rows = r.file().rows();
        assert_eq!(rows[0], DiffRow::Fold { first: 0, len: 143 });
        assert_eq!(rows[1], DiffRow::Context { line: 144, text: "line 144".into() });
        assert_eq!(rows[2], DiffRow::Del { after: 145, text: "line 145".into() });
        assert_eq!(rows[3], DiffRow::Add { line: 145, text: "line 145 changed".into() });
        assert_eq!(rows[4], DiffRow::Context { line: 146, text: "line 146".into() });
        assert_eq!(rows[5], DiffRow::Fold { first: 147, len: 14 });
        assert_eq!(rows.len(), 6);
        assert_eq!((r.file().added_lines, r.file().removed_lines), (1, 1));
    }

    #[test]
    fn a_new_file_is_all_additions_and_marked_added() {
        let r = review_of(None, "a\nb\n");
        assert!(r.file().added);
        assert_eq!(r.file().rows(), vec![DiffRow::Add { line: 1, text: "a".into() }, DiffRow::Add { line: 2, text: "b".into() }]);
    }

    /// A pane whose rows start at screen row 10, column 30, showing from
    /// row `top` of the file.
    fn pane_at(r: &mut Review, top: usize) {
        r.pane = Some(Pane { x: 30, y: 10, width: 60, height: 20, top });
    }

    #[test]
    fn a_click_on_a_fold_opens_it_and_selects_nothing() {
        let before = numbered(30);
        let after = before.replace("line 15\n", "line 15!\n");
        let mut r = review_of(Some(&before), &after);
        assert!(matches!(r.file().rows()[0], DiffRow::Fold { .. }));
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 10);
        assert_eq!(r.file().rows()[0], DiffRow::Context { line: 1, text: "line 1".into() });
        assert_eq!(r.selection(), None);
    }

    #[test]
    fn a_click_selects_one_line_and_a_drag_carries_it_either_way() {
        let mut r = review_of(Some(&numbered(5)), "line 1\nline 2 changed\nline 3 changed\nline 4\nline 5\n");
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 13);
        r.handle_mouse(MouseEventKind::Up(MouseButton::Left), 40, 13);
        assert_eq!(r.selection(), Some((3, 3)), "a click is a one-line selection");

        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 14);
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 12);
        assert_eq!(r.selection(), Some((2, 4)), "dragging upward selects the rows between");
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 2);
        assert_eq!(r.selection(), Some((0, 4)), "past the pane's top it stops at the first row shown");
        r.handle_mouse(MouseEventKind::Up(MouseButton::Left), 40, 2);
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 16);
        assert_eq!(r.selection(), Some((0, 4)), "a move after the release does not drag");
    }

    #[test]
    fn a_click_off_the_diff_selects_nothing_and_the_rows_follow_the_scroll() {
        let mut r = review_of(Some(&numbered(5)), "line 1\nline 2 changed\nline 3 changed\nline 4\nline 5\n");
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 5, 12);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 9);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 29);
        assert_eq!(r.selection(), None, "the tree, the header and below the last row are not lines");
        pane_at(&mut r, 2);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 10);
        assert_eq!(r.selection(), Some((2, 2)));
    }

    /// Rows of `numbered(30)` with line 15 changed: a fold over lines 1–13,
    /// line 14, `−` 15, `+` 15, line 16, a fold over 17–30.
    fn one_change_in_thirty() -> Review {
        let before = numbered(30);
        review_of(Some(&before), &before.replace("line 15\n", "line 15!\n"))
    }

    #[test]
    fn opening_a_fold_keeps_the_selection_on_the_same_lines() {
        let mut r = one_change_in_thirty();
        r.select(3, 3);
        assert_eq!(r.selection_label().unwrap().1, "x.rs · 15");
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 10);
        assert!(!matches!(r.file().rows()[0], DiffRow::Fold { .. }), "the fold above opened");
        assert_eq!(r.selection_label().unwrap().1, "x.rs · 15", "the selection did not move onto line 4");
        assert_eq!(r.selection(), Some((15, 15)), "its drawn row moved down with the lines it covers");
        assert_eq!(r.pane, None, "the pane drawn before the fold opened no longer says what is under a click");
    }

    #[test]
    fn a_drag_that_ends_on_a_fold_takes_every_line_it_hides() {
        let mut r = one_change_in_thirty();
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 14);
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 15);
        assert_eq!(r.selection(), Some((4, 5)));
        assert_eq!(r.selection_label(), Some(("15 lines".into(), "x.rs · 16–30".into())));
    }

    #[test]
    fn space_opens_every_fold_and_keeps_the_selection() {
        let mut r = one_change_in_thirty();
        r.select(3, 3);
        r.handle_key(KeyCode::Char(' '), KeyModifiers::NONE, "");
        assert!(!r.file().has_folds());
        assert_eq!(r.file().rows().len(), 31, "thirty lines and the one removed");
        assert_eq!(r.selection_label().unwrap().1, "x.rs · 15");
    }

    /// Keyboard alone reaches any line: Shift and an arrow selects the top
    /// line shown, the arrows move it, and Shift extends it.
    #[test]
    fn shift_and_the_arrows_select_from_the_keyboard() {
        let mut r = review_of(None, &numbered(40));
        pane_at(&mut r, 0);
        r.handle_key(KeyCode::PageDown, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        assert_eq!(r.selection_label(), Some(("1 line".into(), "x.rs · 11".into())), "the first line shown");
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        assert_eq!(r.selection_label(), Some(("1 line".into(), "x.rs · 13".into())), "the arrows move a selection");
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        assert_eq!(r.selection_label(), Some(("3 lines".into(), "x.rs · 13–15".into())), "Shift extends it");
        r.handle_key(KeyCode::Up, KeyModifiers::SHIFT, "");
        assert_eq!(r.selection_label(), Some(("2 lines".into(), "x.rs · 13–14".into())), "and takes it back");
        for _ in 0..30 {
            r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        }
        assert_eq!(r.selection_label().unwrap().1, "x.rs · 40", "it stops at the last line");
        assert_eq!(r.scroll, 40 - 20, "and the pane follows it");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Up, KeyModifiers::NONE, "");
        assert_eq!((r.selection(), r.scroll), (None, 19), "with nothing selected the arrows scroll again");
    }

    #[test]
    fn the_arrows_and_the_wheel_scroll_the_diff_no_further_than_its_last_pane() {
        let mut r = review_of(None, &numbered(40));
        pane_at(&mut r, 0);
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        r.handle_mouse(MouseEventKind::ScrollDown, 0, 0);
        assert_eq!(r.scroll, 1 + WHEEL_ROWS);
        r.handle_key(KeyCode::PageUp, KeyModifiers::NONE, "");
        assert_eq!(r.scroll, 0);
        for _ in 0..10 {
            r.handle_key(KeyCode::PageDown, KeyModifiers::NONE, "");
        }
        assert_eq!(r.scroll, 40 - 20, "the last pane is full, not one row");
    }

    #[test]
    fn another_file_drops_the_selection_and_the_pane() {
        let mut r = Review::open(
            "r".into(),
            Changeset { files: vec![ChangedFile { path: "a.rs".into(), before: None, after: "a\n".into() }, ChangedFile { path: "b.rs".into(), before: None, after: "b\n".into() }] },
        );
        pane_at(&mut r, 0);
        r.select(0, 0);
        r.handle_key(KeyCode::Tab, KeyModifiers::NONE, "");
        assert_eq!((r.current, r.selection(), r.pane), (1, None, None));
    }

    #[test]
    fn a_selection_and_enter_open_a_comment_on_the_new_file_lines() {
        let before = numbered(5);
        let after = "line 1\nline 2 changed\nline 3 changed\nline 4\nline 5\n";
        let mut r = review_of(Some(&before), after);
        // Rows: 1 ctx, del 2, del 3, add 2, add 3, 4 ctx, 5 ctx (no folds: runs of 1 and 2).
        r.select(4, 3);
        assert_eq!(r.selection(), Some((3, 4)));
        assert_eq!(r.selection_label(), Some(("2 lines".into(), "x.rs · 2–3".into())));

        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        assert!(r.comment.is_some());
        for c in "Use config".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        assert_eq!(r.file().comments, vec![PendingComment { lines: (2, 3), text: "Use config".into() }]);
        assert_eq!(r.selection(), None);
        assert_eq!(r.comment_count(), 1);
    }

    #[test]
    fn approve_waits_for_every_file_to_be_read() {
        let mut r = Review::open(
            "r1".into(),
            Changeset {
                files: vec![
                    ChangedFile { path: "a.rs".into(), before: Some("x\n".into()), after: "y\n".into() },
                    ChangedFile { path: "b.rs".into(), before: None, after: "z\n".into() },
                ],
            },
        );
        assert_eq!(r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, ""), ReviewOutcome::Stay, "grey until every file is read");
        r.mark_read();
        r.handle_key(KeyCode::Tab, KeyModifiers::NONE, "");
        assert_eq!(r.current, 1);
        r.mark_read();
        assert!(r.all_read());
        assert_eq!(r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, ""), ReviewOutcome::Decide(ReviewDecision::Approve));
    }

    /// Without the Kitty protocol `⌃↩` is `↩`, so a bare `↩` on a fully read
    /// review with nothing selected or typed approves.
    #[test]
    fn a_bare_enter_approves_where_ctrl_enter_cannot_be_told_apart() {
        let mut r = review_of(Some("x\n"), "y\n");
        assert_eq!(r.handle_key(KeyCode::Enter, KeyModifiers::NONE, ""), ReviewOutcome::Stay, "not before every file is read");
        r.mark_read();
        assert_eq!(r.handle_key(KeyCode::Enter, KeyModifiers::NONE, ""), ReviewOutcome::Decide(ReviewDecision::Approve));
    }

    #[test]
    fn comments_send_before_an_approve_and_a_typed_message_is_a_general_comment() {
        let mut r = review_of(Some("x\n"), "y\n");
        r.mark_read();
        r.select(0, 1);
        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        for c in "no".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        let ReviewOutcome::Decide(ReviewDecision::Comment { comments }) = r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, "and rename it") else {
            panic!("comments must send before an approve")
        };
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].path, "src/x.rs");
        assert_eq!(comments[1], ReviewComment { path: String::new(), lines: (0, 0), text: "and rename it".into() });
    }

    #[test]
    fn escape_clears_a_selection_then_asks_before_discarding() {
        let mut r = review_of(Some("x\n"), "y\n");
        r.select(0, 0);
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert_eq!(r.selection(), None);
        assert!(!r.confirm_discard);
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert!(r.confirm_discard);
        r.handle_key(KeyCode::Char('1'), KeyModifiers::NONE, "");
        assert!(!r.confirm_discard, "1 keeps reviewing");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert_eq!(r.handle_key(KeyCode::Char('2'), KeyModifiers::NONE, ""), ReviewOutcome::Decide(ReviewDecision::Discard));
    }
}
