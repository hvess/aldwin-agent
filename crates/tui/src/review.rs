//! The review's state (ADR 0009 §4): the files of a staged changeset as the
//! design draws them — a tree with reading progress, a diff with folded
//! runs, a cursor, a selection, and the comments left on it — and what each
//! key does to that.
//!
//! Pure state, no drawing: `ui::review` reads this and `App` drives it, so
//! every rule here is testable without a terminal.

use aldwin_core::{Changeset, ReviewComment, ReviewDecision};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

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

    pub fn is_fold(&self) -> bool {
        matches!(self, DiffRow::Fold { .. })
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
    /// The rows as drawn: folds collapsed to one row each.
    pub fn rows(&self) -> Vec<DiffRow> {
        let mut out = Vec::with_capacity(self.unfolded.len());
        let mut i = 0;
        let mut folds = self.folds.iter().peekable();
        while i < self.unfolded.len() {
            match folds.peek() {
                Some(&&(start, len)) if start == i => {
                    out.push(DiffRow::Fold { first: start, len });
                    i += len;
                    folds.next();
                }
                _ => {
                    out.push(self.unfolded[i].clone());
                    i += 1;
                }
            }
        }
        out
    }

    /// Opens the fold starting at unfolded row `first`.
    pub fn expand(&mut self, first: usize) {
        self.folds.retain(|&(start, _)| start != first);
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
    /// Row index into `files[current].rows()`.
    pub cursor:    usize,
    /// Where a selection started; `Some` means rows from `anchor` to
    /// `cursor` are selected.
    pub anchor:    Option<usize>,
    /// First row of the current file's pane — kept by the drawing side so
    /// the cursor stays in view.
    pub scroll:    usize,
    pub comment:   Option<CommentDraft>,
    /// `⎋` with nothing selected asks before dropping the changes.
    pub confirm_discard: bool,
    /// `?` toggles the key list in the footer.
    pub keys_shown: bool,
}

impl Review {
    pub fn open(review_id: String, changeset: Changeset) -> Self {
        let files = changeset.files.into_iter().map(|f| file_from(&f.path, f.before.as_deref(), &f.after)).collect();
        Self { review_id, files, current: 0, cursor: 0, anchor: None, scroll: 0, comment: None, confirm_discard: false, keys_shown: false }
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

    /// The rows selected, as an inclusive index range into the current
    /// file's rows.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    /// `router.rs · 144–145` and `2 lines` — what the comment field's label
    /// names.
    pub fn selection_label(&self) -> Option<(String, String)> {
        let (from, to) = self.selection()?;
        let rows = self.file().rows();
        let lines = self.line_range(&rows, from, to)?;
        let name = self.file().path.rsplit('/').next().unwrap_or(&self.file().path).to_string();
        let where_ = if lines.0 == lines.1 { format!("{name} · {}", lines.0) } else { format!("{name} · {}–{}", lines.0, lines.1) };
        let n = to - from + 1;
        Some((format!("{n} {}", if n == 1 { "line" } else { "lines" }), where_))
    }

    fn line_range(&self, rows: &[DiffRow], from: usize, to: usize) -> Option<(usize, usize)> {
        let anchors: Vec<usize> = rows.get(from..=to)?.iter().filter_map(DiffRow::anchor).collect();
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
                        if let Some((from, to)) = self.selection() {
                            let rows = self.file().rows();
                            if let Some(lines) = self.line_range(&rows, from, to) {
                                self.file_mut().comments.push(PendingComment { lines, text });
                            }
                        }
                    }
                    self.comment = None;
                    self.anchor = None;
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

        let rows = self.file().rows();
        let last = rows.len().saturating_sub(1);
        let shift = modifiers.contains(KeyModifiers::SHIFT);
        match code {
            KeyCode::Up | KeyCode::Down => {
                if shift && self.anchor.is_none() {
                    self.anchor = Some(self.cursor);
                } else if !shift {
                    self.anchor = None;
                }
                let step = |c: usize| if code == KeyCode::Up { c.saturating_sub(1) } else { (c + 1).min(last) };
                let mut next = step(self.cursor);
                // A fold is not a line; step over it unless it is all there is.
                while rows.get(next).is_some_and(DiffRow::is_fold) && next != self.cursor {
                    let further = step(next);
                    if further == next {
                        break;
                    }
                    next = further;
                }
                self.cursor = next;
            }
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(10),
            KeyCode::PageDown => self.cursor = (self.cursor + 10).min(last),
            KeyCode::Tab | KeyCode::Right => self.go_to_file((self.current + 1) % self.files.len().max(1)),
            KeyCode::BackTab | KeyCode::Left => self.go_to_file((self.current + self.files.len().max(1) - 1) % self.files.len().max(1)),
            KeyCode::Char(' ') if general.is_empty() => {
                if let Some(DiffRow::Fold { first, .. }) = rows.get(self.cursor) {
                    let first = *first;
                    self.file_mut().expand(first);
                }
            }
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
                if self.anchor.is_some() {
                    self.comment = Some(CommentDraft::default());
                } else if let Some(decision) = self.comments_decision(general) {
                    return ReviewOutcome::Decide(decision);
                } else if self.all_read() {
                    return ReviewOutcome::Decide(ReviewDecision::Approve);
                }
            }
            KeyCode::Esc => {
                if self.anchor.is_some() {
                    self.anchor = None;
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
        self.cursor = 0;
        self.anchor = None;
        self.scroll = 0;
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

    #[test]
    fn a_fold_expands_on_space_and_the_cursor_steps_over_folds() {
        let before = numbered(30);
        let after = before.replace("line 15\n", "line 15!\n");
        let mut r = review_of(Some(&before), &after);
        assert!(r.file().rows()[0].is_fold());
        // Down from the fold lands on the first real row past it.
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        assert_eq!(r.cursor, 1);
        r.cursor = 0;
        r.handle_key(KeyCode::Char(' '), KeyModifiers::NONE, "");
        assert_eq!(r.file().rows()[0], DiffRow::Context { line: 1, text: "line 1".into() });
    }

    #[test]
    fn shift_arrows_select_and_enter_opens_a_comment_on_the_new_file_lines() {
        let before = numbered(5);
        let after = "line 1\nline 2 changed\nline 3 changed\nline 4\nline 5\n";
        let mut r = review_of(Some(&before), after);
        // Rows: 1 ctx, del 2, del 3, add 2, add 3, 4 ctx, 5 ctx (no folds: runs of 1 and 2).
        r.cursor = 3;
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        assert_eq!(r.selection(), Some((3, 4)));
        assert_eq!(r.selection_label(), Some(("2 lines".into(), "x.rs · 2–3".into())));

        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        assert!(r.comment.is_some());
        for c in "Use config".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        assert_eq!(r.file().comments, vec![PendingComment { lines: (2, 3), text: "Use config".into() }]);
        assert_eq!(r.anchor, None);
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
        r.anchor = Some(0);
        r.cursor = 1;
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
        r.anchor = Some(0);
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert_eq!(r.anchor, None);
        assert!(!r.confirm_discard);
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert!(r.confirm_discard);
        r.handle_key(KeyCode::Char('1'), KeyModifiers::NONE, "");
        assert!(!r.confirm_discard, "1 keeps reviewing");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert_eq!(r.handle_key(KeyCode::Char('2'), KeyModifiers::NONE, ""), ReviewOutcome::Decide(ReviewDecision::Discard));
    }
}
