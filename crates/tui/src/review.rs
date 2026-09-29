//! The review's state (ADR 0009 §4): a changeset's files, folded diffs,
//! reading progress, the mouse selection (ADR 0010) and pending comments,
//! and what each key and click does.
//!
//! No drawing here: `ui::review` reads this and `App` drives it.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use aldwin_core::{Changeset, Question, ReviewComment, ReviewDecision};
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use similar::{capture_diff_slices_deadline, Algorithm, Change, ChangeTag};

use crate::draft::Draft;
use crate::list::{List, ListOutcome, ListRow};
use crate::log::plural;
use crate::scroll::WHEEL_ROWS;

/// Rows `PgUp` and `PgDn` scroll the diff.
const PAGE_ROWS: usize = 10;

/// Where the diff's rows were drawn last frame. Must be dropped whenever the
/// drawn rows change (another file, an opened fold) until the next draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pane {
    /// Screen cell of the first row's left edge, below the pane's header;
    /// with `y` and `width`.
    pub x: u16,
    pub y: u16,
    pub width: u16,
    /// The drawn row each screen row shows, top to bottom; a wrapped line
    /// (baseline `long-diff-lines-wrap`) takes several.
    pub lines: Vec<usize>,
    /// The first drawn row shown.
    pub top: usize,
    /// The last drawn row shown whole.
    pub bottom: usize,
    /// The largest `top` that still fills the pane.
    pub last_top: usize,
}

/// Where the tree's rows were drawn last frame; a click on a file row shows
/// that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tree {
    /// Screen cell of the tree's top-left corner; with `y` and `width`.
    pub x: u16,
    pub y: u16,
    pub width: u16,
    /// The file each screen row from `y` shows; `None` for a blank, the
    /// dots or a folder.
    pub files: Vec<Option<usize>>,
}

/// Unchanged lines kept on each side of a change; the rest fold. One, as
/// the frame draws it.
const CONTEXT: usize = 1;

/// One row of a file's diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    /// A folded run of unchanged lines: `first` indexes the unfolded rows,
    /// `len` is how many it hides.
    Fold {
        first: usize,
        len: usize,
    },
    /// Unchanged, with its new-file line number.
    Context {
        line: usize,
        text: String,
    },
    Add {
        line: usize,
        text: String,
    },
    /// Removed; `after` is the nearest new-file line after it, for a
    /// comment to anchor on.
    Del {
        after: usize,
        text: String,
    },
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

/// The comment field's label for the selection: `2 lines`, `router.rs`,
/// `144–145`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectionLabel {
    /// How many lines are selected, as a phrase.
    pub lines: String,
    /// The file's name, without its directory.
    pub file: String,
    /// New-file line numbers, one or `first–last`.
    pub range: String,
}

/// A comment left on a run of lines, not yet sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingComment {
    /// Inclusive new-file line numbers.
    pub lines: (usize, usize),
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFile {
    pub path: String,
    /// A new file; the tree draws ` +` beside it.
    pub added: bool,
    /// Every row, unfolded.
    unfolded: Vec<DiffRow>,
    /// Unexpanded folds over `unfolded`, as (start row, length), in order.
    folds: Vec<(usize, usize)>,
    /// Each drawn row as the unfolded rows it stands for: `(start, len,
    /// folded)`, in order. Derived from `folds`: [`ReviewFile::refold`]
    /// rebuilds it, a walk of the whole file, only when a fold opens; a
    /// frame, a scroll or a selection looks rows up in it.
    drawn: Vec<(usize, usize, bool)>,
    /// The `+11` of `+11 −2`; `removed_lines` is the `−2`.
    pub added_lines: usize,
    pub removed_lines: usize,
    /// Scrolled to the bottom; approve (`⌃↩`) waits for every file.
    pub read: bool,
    pub comments: Vec<PendingComment>,
}

impl ReviewFile {
    /// Rebuilds `drawn` from `unfolded` and `folds`.
    fn refold(&mut self) {
        let mut i = 0;
        let mut folds = self.folds.iter().peekable();
        self.drawn = std::iter::from_fn(|| {
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
        .collect();
    }

    /// How many rows are drawn: folds collapsed to one row each.
    pub(crate) fn row_count(&self) -> usize {
        self.drawn.len()
    }

    /// Drawn row `row`: borrowed, or built for a fold.
    pub(crate) fn row(&self, row: usize) -> Option<Cow<'_, DiffRow>> {
        let &(start, len, folded) = self.drawn.get(row)?;
        Some(if folded {
            Cow::Owned(DiffRow::Fold { first: start, len })
        } else {
            Cow::Borrowed(&self.unfolded[start])
        })
    }

    /// The rows as drawn, cloned.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<DiffRow> {
        (0..self.row_count())
            .filter_map(|i| self.row(i).map(Cow::into_owned))
            .collect()
    }

    /// The unfolded rows drawn row `row` stands for, first and last.
    fn span_of(&self, row: usize) -> Option<(usize, usize)> {
        self.drawn
            .get(row)
            .map(|&(start, len, _)| (start, start + len - 1))
    }

    /// The drawn row that shows unfolded row `unfolded` — its own, or the
    /// fold hiding it; 0 past the end.
    fn row_of(&self, unfolded: usize) -> usize {
        let row = self
            .drawn
            .partition_point(|&(start, len, _)| start + len <= unfolded);
        if row < self.drawn.len() {
            row
        } else {
            0
        }
    }

    /// Opens the fold starting at unfolded row `first`.
    pub fn expand(&mut self, first: usize) {
        self.folds.retain(|&(start, _)| start != first);
        self.refold();
    }

    /// Opens every fold.
    pub(crate) fn expand_all(&mut self) {
        self.folds.clear();
        self.refold();
    }

    pub(crate) fn has_folds(&self) -> bool {
        !self.folds.is_empty()
    }
}

/// A selection in unfolded rows, so opening a fold (which renumbers drawn
/// rows) cannot move it. Each end is a drawn row's span, so ending on a
/// fold takes every line it hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    /// Where the press landed.
    anchor: (usize, usize),
    /// Where the drag is now.
    head: (usize, usize),
}

impl Selection {
    /// The unfolded rows covered, first and last.
    fn lines(self) -> (usize, usize) {
        (
            self.anchor.0.min(self.head.0),
            self.anchor.1.max(self.head.1),
        )
    }
}

/// A review's place in a comment round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Round {
    /// Nothing sent: the developer reads, comments and decides.
    Open,
    /// Comments sent and no follow-up turn started: before a `run` they
    /// are answered within the same turn, at its end by the follow-up.
    Sent,
    /// The follow-up turn is addressing them.
    Addressing,
}

/// What one key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewOutcome {
    /// Handled within the review; nothing to send.
    Stay,
    /// A decision for the caller to send.
    Decide(ReviewDecision),
}

/// The full-window review of one changeset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    /// The review's id, echoed back with the decision.
    pub review_id: String,
    /// One per changed file, in order. Never empty: `open` refuses an empty
    /// changeset and nothing removes one; `file` relies on it.
    files: Vec<ReviewFile>,
    /// Which of `files` is on screen.
    pub current: usize,
    /// The lines selected for a comment. There is no line cursor (ADR
    /// 0010); a row is marked only inside a selection.
    selected: Option<Selection>,
    /// A button is down on the diff; the selection follows the pointer.
    dragging: bool,
    /// First drawn row of the pane. Keys and wheel clamp it to the last full
    /// pane once drawn; the drawing side clamps it again and writes it back.
    pub scroll: usize,
    /// Where the last frame drew the diff's rows; clicks are measured
    /// against it.
    pub(crate) pane: Option<Pane>,
    /// Where the last frame drew the tree; the same.
    pub(crate) tree: Option<Tree>,
    /// Where the review is in a comment round. While not `Open` no line can
    /// be selected and no decision made, until the agent's next changeset
    /// replaces this review (`carry_from`) or a turn end closes it
    /// (`closes_at_turn_end`).
    round: Round,
    /// The comment field's text; `esc` closes the field but keeps it.
    pub(crate) comment: Draft,
    /// The discard question's list, while open (`esc` with nothing
    /// selected).
    pub(crate) confirm: Option<List>,
    /// `?` toggles the key list in the footer.
    pub keys_shown: bool,
    /// The header's title: the request the round began with, `Changes`
    /// until `App` sets it when the review opens; `carry_from` keeps it,
    /// so the echoed comments of a later round never replace it.
    pub(crate) title: String,
}

impl Review {
    /// Opens on the first file, nothing selected or read; `None` for a
    /// changeset with no files.
    pub fn open(review_id: String, changeset: Changeset) -> Option<Self> {
        if changeset.files.is_empty() {
            return None;
        }
        let files = changeset
            .files
            .into_iter()
            .map(|f| file_from(&f.path, f.before.as_deref(), &f.after))
            .collect();
        Some(Self {
            review_id,
            files,
            current: 0,
            selected: None,
            dragging: false,
            scroll: 0,
            pane: None,
            tree: None,
            round: Round::Open,
            comment: Draft::default(),
            confirm: None,
            keys_shown: false,
            title: "Changes".into(),
        })
    }

    /// Whether the comments are with the agent and the next changeset is
    /// awaited.
    pub(crate) fn waiting(&self) -> bool {
        self.round != Round::Open
    }

    /// Enters the waiting state once the comments are sent: the selection
    /// and the discard question go, the comments stay drawn.
    pub(crate) fn await_agent(&mut self) {
        self.round = Round::Sent;
        self.selected = None;
        self.dragging = false;
        self.confirm = None;
    }

    /// The follow-up turn carrying the comments has started (ADR 0009 §4).
    pub(crate) fn follow_up_started(&mut self) {
        if self.round == Round::Sent {
            self.round = Round::Addressing;
        }
    }

    /// Whether a turn ending (`finished`: with `EndTurn`) closes this
    /// review. Comments sent before a `run` come back within the turn, and
    /// the turn-end review of what is still staged replaces this one before
    /// the turn ends. A finished turn whose end review took comments starts
    /// the follow-up, so the review waits on; a stopped or failed one starts
    /// none, and the follow-up's own end means no changeset came, so the
    /// agent's reply must not stay hidden behind the review.
    pub(crate) fn closes_at_turn_end(&self, finished: bool) -> bool {
        match self.round {
            Round::Open => false,
            Round::Sent => !finished,
            Round::Addressing => true,
        }
    }

    /// Takes over from the review this one replaces (the agent's next round
    /// after comments): the title, a file whose diff did not change stays
    /// read, the file on screen stays on screen, and the key list stays as
    /// it was.
    pub(crate) fn carry_from(&mut self, previous: &Review) {
        self.title = previous.title.clone();
        for file in &mut self.files {
            file.read = previous
                .files
                .iter()
                .any(|p| p.read && p.path == file.path && p.unfolded == file.unfolded);
        }
        let shown = &previous.file().path;
        if let Some(i) = self.files.iter().position(|f| &f.path == shown) {
            self.current = i;
            if self.files[i].unfolded == previous.file().unfolded {
                self.scroll = previous.scroll;
            }
        }
        self.keys_shown = previous.keys_shown;
    }

    /// Every file in the review, in the changeset's order; never empty.
    pub(crate) fn files(&self) -> &[ReviewFile] {
        &self.files
    }

    /// The file on screen.
    pub fn file(&self) -> &ReviewFile {
        &self.files[self.current.min(self.files.len().saturating_sub(1))]
    }

    fn file_mut(&mut self) -> &mut ReviewFile {
        let i = self.current.min(self.files.len().saturating_sub(1));
        &mut self.files[i]
    }

    /// How many files have been read.
    pub fn files_read(&self) -> usize {
        self.files.iter().filter(|f| f.read).count()
    }

    /// Whether every file has been read.
    pub fn all_read(&self) -> bool {
        self.files.iter().all(|f| f.read)
    }

    /// The comment field is open exactly when lines are selected, as frame H
    /// draws it.
    pub(crate) fn commenting(&self) -> bool {
        self.selected.is_some()
    }

    /// The comments left across every file.
    pub fn comment_count(&self) -> usize {
        self.files.iter().map(|f| f.comments.len()).sum()
    }

    /// The drawn rows selected, as an inclusive index range into the
    /// current file's rows.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let (first, last) = self.selected?.lines();
        Some((self.file().row_of(first), self.file().row_of(last)))
    }

    /// Selects drawn rows `from` through `to`, as a press and drag would.
    pub fn select(&mut self, from: usize, to: usize) {
        let file = self.file();
        if let (Some(anchor), Some(head)) = (file.span_of(from), file.span_of(to)) {
            self.selected = Some(Selection { anchor, head });
        }
    }

    /// An arrow on a selection: `extend` (Shift) moves its head a row;
    /// otherwise it becomes the one line past the head. With nothing
    /// selected, selects the first line shown.
    fn step_selection(&mut self, up: bool, extend: bool) {
        let rows = self.file().row_count();
        let Some(selected) = self.selected else {
            let top = self.scroll.min(rows.saturating_sub(1));
            self.select(top, top);
            return;
        };
        let head = self.file().row_of(selected.head.0);
        let next = if up {
            head.saturating_sub(1)
        } else {
            (head + 1).min(rows.saturating_sub(1))
        };
        let Some(span) = self.file().span_of(next) else {
            return;
        };
        self.selected = Some(if extend {
            Selection {
                head: span,
                ..selected
            }
        } else {
            Selection {
                anchor: span,
                head: span,
            }
        });
        self.keep_in_view(next);
    }

    /// Scrolls just enough to show drawn row `row`, using the last frame's
    /// whole-row count.
    fn keep_in_view(&mut self, row: usize) {
        let shown = self.pane.as_ref().map_or(1, |p| p.bottom + 1 - p.top);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + shown {
            self.scroll = row + 1 - shown;
        }
    }

    /// Scrolls by `delta` rows, never past the last full pane; before the
    /// first draw only the drawing side clamps.
    fn scroll_by(&mut self, delta: isize) {
        let last_top = self.pane.as_ref().map_or(usize::MAX, |p| p.last_top);
        self.scroll = self.scroll.saturating_add_signed(delta).min(last_top);
    }

    /// Must be called when the drawn rows change: drops the stale pane, so
    /// no click is honoured until the next draw.
    fn rows_changed(&mut self) {
        self.pane = None;
    }

    /// One mouse event (ADR 0010). A press on the tree shows that file; on
    /// the diff it selects a line or opens a fold, and a drag extends. The
    /// wheel scrolls anywhere. Ignored while the discard question is open;
    /// while waiting, nothing is selected. An open comment field keeps its
    /// text on a click.
    pub fn handle_mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        if self.confirm.is_some() {
            return;
        }
        match kind {
            MouseEventKind::ScrollUp => self.scroll_by(-(WHEEL_ROWS as isize)),
            MouseEventKind::ScrollDown => self.scroll_by(WHEEL_ROWS as isize),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(i) = self.file_at(column, row) {
                    if i != self.current {
                        self.go_to_file(i);
                    }
                    return;
                }
                let Some(at) = self.row_at(column, row, false) else {
                    return;
                };
                if let Some(DiffRow::Fold { first, .. }) = self.file().row(at).as_deref() {
                    let first = *first;
                    self.file_mut().expand(first);
                    self.rows_changed();
                    return;
                }
                if self.waiting() {
                    return;
                }
                self.select(at, at);
                self.dragging = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                // Past the pane's edge, clamp to the row shown there.
                let head = self
                    .row_at(column, row, true)
                    .and_then(|at| self.file().span_of(at));
                if let (Some(head), Some(selected)) = (head, &mut self.selected) {
                    selected.head = head;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            _ => {}
        }
    }

    /// The file whose tree row is under a screen cell, or `None` off a file
    /// row.
    fn file_at(&self, column: u16, row: u16) -> Option<usize> {
        let tree = self.tree.as_ref()?;
        if !(tree.x..tree.x + tree.width).contains(&column) {
            return None;
        }
        tree.files
            .get(row.checked_sub(tree.y)? as usize)
            .copied()
            .flatten()
    }

    /// The drawn row under a screen cell, or `None` off the diff. With
    /// `clamp`, any point lands on the nearest row shown.
    fn row_at(&self, column: u16, row: u16, clamp: bool) -> Option<usize> {
        let pane = self.pane.as_ref()?;
        let shown = pane.lines.len();
        let inside = (pane.x..pane.x + pane.width).contains(&column)
            && (pane.y..pane.y + shown as u16).contains(&row);
        if shown == 0 || !(inside || clamp) {
            return None;
        }
        Some(pane.lines[(row.saturating_sub(pane.y) as usize).min(shown - 1)])
    }

    /// The comment field's label, while a selection is held.
    pub fn selection_label(&self) -> Option<SelectionLabel> {
        let (first, last) = self.selected?.lines();
        let lines = self.line_range(first, last)?;
        let name = self
            .file()
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&self.file().path)
            .to_string();
        let range = if lines.0 == lines.1 {
            lines.0.to_string()
        } else {
            format!("{}–{}", lines.0, lines.1)
        };
        Some(SelectionLabel {
            lines: plural(last - first + 1, "line"),
            file: name,
            range,
        })
    }

    /// The new-file lines unfolded rows `first` through `last` anchor to.
    fn line_range(&self, first: usize, last: usize) -> Option<(usize, usize)> {
        let anchors: Vec<usize> = self
            .file()
            .unfolded
            .get(first..=last)?
            .iter()
            .filter_map(DiffRow::anchor)
            .collect();
        Some((*anchors.iter().min()?, *anchors.iter().max()?))
    }

    /// Called by the drawing side once the current file's bottom is on
    /// screen.
    pub fn mark_read(&mut self) {
        self.file_mut().read = true;
    }

    /// Files not yet read to the bottom; approve waits for them.
    pub fn unread(&self) -> usize {
        self.files.len() - self.files_read()
    }

    /// On a refused approve, shows the first unread file, unless the file
    /// on screen is unread itself.
    fn show_unread(&mut self) {
        if self.file().read {
            if let Some(i) = self.files.iter().position(|f| !f.read) {
                self.go_to_file(i);
            }
        }
    }

    /// The question `esc` asks before the changes are dropped.
    pub fn discard_question(&self) -> Question {
        Question {
            question: "Discard these changes?".into(),
            detail: "Nothing has been written. The agent is told.".into(),
            options: vec![
                "Keep reviewing".into(),
                format!("Discard {}", plural(self.files.len(), "file")),
            ],
        }
    }

    /// Every pending comment, plus non-blank `general`, as a decision;
    /// `None` when there are none.
    fn comments_decision(&self, general: &str) -> Option<ReviewDecision> {
        let mut comments: Vec<ReviewComment> = self
            .files
            .iter()
            .flat_map(|f| {
                f.comments.iter().map(move |c| ReviewComment {
                    path: f.path.clone(),
                    lines: c.lines,
                    text: c.text.clone(),
                })
            })
            .collect();
        if !general.trim().is_empty() {
            comments.push(ReviewComment {
                path: String::new(),
                lines: (0, 0),
                text: general.trim().to_string(),
            });
        }
        (!comments.is_empty()).then_some(ReviewDecision::Comment { comments })
    }

    /// One key. `general` is the "Ask for a change" field's text, which the
    /// caller edits; the keys that send read it. While waiting, the keys
    /// that select or decide do nothing, and `esc` is the caller's to stop
    /// the turn with.
    pub fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        general: &str,
    ) -> ReviewOutcome {
        if let Some(list) = &mut self.confirm {
            return match list.handle_key(code, modifiers) {
                ListOutcome::Stay => ReviewOutcome::Stay,
                ListOutcome::Chose(1) => ReviewOutcome::Decide(ReviewDecision::Discard),
                ListOutcome::Close | ListOutcome::Chose(_) => {
                    self.confirm = None;
                    ReviewOutcome::Stay
                }
            };
        }
        let shift = modifiers.contains(KeyModifiers::SHIFT);
        match code {
            KeyCode::Enter | KeyCode::Esc if self.waiting() => {}
            KeyCode::Up | KeyCode::Down if shift && self.waiting() => {}
            // Shift+arrow selects (HIG, "Keyboards"; ADR 0010). Plain arrows
            // scroll until a line is selected, then move it.
            KeyCode::Up | KeyCode::Down if shift || self.commenting() => {
                self.step_selection(code == KeyCode::Up, shift)
            }
            KeyCode::Up => self.scroll_by(-1),
            KeyCode::Down => self.scroll_by(1),
            KeyCode::PageUp => self.scroll_by(-(PAGE_ROWS as isize)),
            KeyCode::PageDown => self.scroll_by(PAGE_ROWS as isize),
            KeyCode::Tab => self.go_to_file((self.current + 1) % self.files.len().max(1)),
            KeyCode::BackTab => self.previous_file(),
            // With the comment field open, add its comment first so it is
            // sent too.
            KeyCode::Enter if modifiers.contains(KeyModifiers::CONTROL) => {
                if self.commenting() {
                    self.add_comment();
                }
                return self.act(general);
            }
            KeyCode::Enter if self.commenting() => self.add_comment(),
            KeyCode::Esc if self.commenting() => {
                self.selected = None;
                self.dragging = false;
            }
            // Must precede the arms below: in the comment field `←→` move
            // the caret and Space and `?` are typed.
            code if self.commenting() => {
                self.comment.edit(code, modifiers);
            }
            KeyCode::Right => self.go_to_file((self.current + 1) % self.files.len().max(1)),
            KeyCode::Left => self.previous_file(),
            // The keyboard's way to open folds, needed where the mouse is
            // not captured.
            KeyCode::Char(' ') if general.is_empty() && self.file().has_folds() => {
                self.file_mut().expand_all();
                self.rows_changed();
            }
            KeyCode::Char('?') if general.is_empty() => self.keys_shown = !self.keys_shown,
            // Must act like `⌃↩`: without the Kitty protocol the terminal
            // cannot tell them apart, and the review could never be approved.
            KeyCode::Enter => return self.act(general),
            KeyCode::Esc => {
                let rows = self.discard_question().options;
                self.confirm = Some(List::new(rows.into_iter().map(ListRow::new).collect()));
            }
            _ => {}
        }
        ReviewOutcome::Stay
    }

    /// `⌃↩`: send the comments (typed `general` counts as one), else approve
    /// if every file is read, else show an unread file.
    fn act(&mut self, general: &str) -> ReviewOutcome {
        if let Some(decision) = self.comments_decision(general) {
            return ReviewOutcome::Decide(decision);
        }
        if self.all_read() {
            return ReviewOutcome::Decide(ReviewDecision::Approve);
        }
        self.show_unread();
        ReviewOutcome::Stay
    }

    /// `↩` in the comment field: attaches the comment to the selection and
    /// closes the field.
    fn add_comment(&mut self) {
        let text = self.comment.take().trim().to_string();
        let lines = self
            .selected
            .and_then(|s| self.line_range(s.lines().0, s.lines().1));
        if let (false, Some(lines)) = (text.is_empty(), lines) {
            self.file_mut()
                .comments
                .push(PendingComment { lines, text });
        }
        self.selected = None;
    }

    fn previous_file(&mut self) {
        let n = self.files.len().max(1);
        self.go_to_file((self.current + n - 1) % n);
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

/// A file's diff, folded. The common prefix and suffix are trimmed before
/// the diff, which costs O((n + m) · d) for d changed lines ([`line_diff`]).
fn file_from(path: &str, before: Option<&str>, after: &str) -> ReviewFile {
    let before_lines: Vec<&str> = before.map(|b| b.lines().collect()).unwrap_or_default();
    let after_lines: Vec<&str> = after.lines().collect();

    let prefix = before_lines
        .iter()
        .zip(&after_lines)
        .take_while(|(a, b)| a == b)
        .count();
    let max_suffix = before_lines.len().min(after_lines.len()) - prefix;
    let suffix = before_lines
        .iter()
        .rev()
        .zip(after_lines.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();

    let mut unfolded: Vec<DiffRow> = Vec::with_capacity(after_lines.len() + before_lines.len());
    for (i, text) in after_lines[..prefix].iter().enumerate() {
        unfolded.push(DiffRow::Context {
            line: i + 1,
            text: (*text).to_string(),
        });
    }
    let mid_a = &before_lines[prefix..before_lines.len() - suffix];
    let mid_b = &after_lines[prefix..after_lines.len() - suffix];
    let mut line = prefix + 1;
    let mut added_lines = 0;
    let mut removed_lines = 0;
    let mut pending_dels: Vec<String> = Vec::new();
    for change in line_diff(mid_a, mid_b) {
        let text = change.value();
        match change.tag() {
            ChangeTag::Equal => {
                for d in pending_dels.drain(..) {
                    unfolded.push(DiffRow::Del {
                        after: line,
                        text: d,
                    });
                }
                unfolded.push(DiffRow::Context {
                    line,
                    text: text.to_string(),
                });
                line += 1;
            }
            ChangeTag::Delete => {
                removed_lines += 1;
                pending_dels.push(text.to_string());
            }
            ChangeTag::Insert => {
                for d in pending_dels.drain(..) {
                    unfolded.push(DiffRow::Del {
                        after: line,
                        text: d,
                    });
                }
                added_lines += 1;
                unfolded.push(DiffRow::Add {
                    line,
                    text: text.to_string(),
                });
                line += 1;
            }
        }
    }
    for d in pending_dels.drain(..) {
        unfolded.push(DiffRow::Del {
            after: line,
            text: d,
        });
    }
    for text in &after_lines[after_lines.len() - suffix..] {
        unfolded.push(DiffRow::Context {
            line,
            text: (*text).to_string(),
        });
        line += 1;
    }

    let folds = fold_runs(&unfolded);
    let mut file = ReviewFile {
        path: path.to_string(),
        added: before.is_none(),
        unfolded,
        folds,
        drawn: Vec::new(),
        added_lines,
        removed_lines,
        read: false,
        comments: Vec::new(),
    };
    file.refold();
    file
}

/// Folds context runs, keeping `CONTEXT` lines beside each change (a run at
/// the file's start or end keeps them on its changed side only). A fold
/// hides at least two lines.
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

/// How long one file's diff may look for the fewest changed lines: the
/// review opens on the UI thread. Past it the diff is still correct, with
/// more lines marked changed than needed.
const DIFF_BUDGET: Duration = Duration::from_millis(200);

/// Myers' diff of two files' lines: O((n + m) · d) time and O(n + m) space
/// for d changed lines, so a small edit to a large file stays cheap.
fn line_diff<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Change<&'a str>> {
    let deadline = Instant::now() + DIFF_BUDGET;
    capture_diff_slices_deadline(Algorithm::Myers, a, b, Some(deadline))
        .iter()
        .flat_map(|op| op.iter_changes(a, b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_core::ChangedFile;

    fn label(lines: &str, file: &str, range: &str) -> Option<SelectionLabel> {
        Some(SelectionLabel {
            lines: lines.into(),
            file: file.into(),
            range: range.into(),
        })
    }

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    fn review_of(before: Option<&str>, after: &str) -> Review {
        Review::open(
            "r1".into(),
            Changeset {
                files: vec![ChangedFile {
                    path: "src/x.rs".into(),
                    before: before.map(str::to_string),
                    after: after.into(),
                }],
            },
        )
        .expect("a changeset with files")
    }

    /// The frame's shape.
    #[test]
    fn unchanged_runs_fold_with_one_line_of_context() {
        let before = numbered(160);
        let after = before.replace("line 145\n", "line 145 changed\n");
        let r = review_of(Some(&before), &after);
        let rows = r.file().rows();
        assert_eq!(rows[0], DiffRow::Fold { first: 0, len: 143 });
        assert_eq!(
            rows[1],
            DiffRow::Context {
                line: 144,
                text: "line 144".into()
            }
        );
        assert_eq!(
            rows[2],
            DiffRow::Del {
                after: 145,
                text: "line 145".into()
            }
        );
        assert_eq!(
            rows[3],
            DiffRow::Add {
                line: 145,
                text: "line 145 changed".into()
            }
        );
        assert_eq!(
            rows[4],
            DiffRow::Context {
                line: 146,
                text: "line 146".into()
            }
        );
        assert_eq!(
            rows[5],
            DiffRow::Fold {
                first: 147,
                len: 14
            }
        );
        assert_eq!(rows.len(), 6);
        assert_eq!((r.file().added_lines, r.file().removed_lines), (1, 1));
    }

    #[test]
    fn a_new_file_is_all_additions_and_marked_added() {
        let r = review_of(None, "a\nb\n");
        assert!(r.file().added);
        assert_eq!(
            r.file().rows(),
            vec![
                DiffRow::Add {
                    line: 1,
                    text: "a".into()
                },
                DiffRow::Add {
                    line: 2,
                    text: "b".into()
                }
            ]
        );
    }

    /// A 20-row, unwrapped pane at screen (30, 10), showing from drawn row
    /// `top`.
    fn pane_at(r: &mut Review, top: usize) {
        let rows = r.file().rows().len();
        let lines: Vec<usize> = (top..rows.min(top + 20)).collect();
        r.pane = Some(Pane {
            x: 30,
            y: 10,
            width: 60,
            bottom: lines.last().copied().unwrap_or(top),
            lines,
            top,
            last_top: rows.saturating_sub(20),
        });
    }

    #[test]
    fn a_click_on_a_fold_opens_it_and_selects_nothing() {
        let before = numbered(30);
        let after = before.replace("line 15\n", "line 15!\n");
        let mut r = review_of(Some(&before), &after);
        assert!(matches!(r.file().rows()[0], DiffRow::Fold { .. }));
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 10);
        assert_eq!(
            r.file().rows()[0],
            DiffRow::Context {
                line: 1,
                text: "line 1".into()
            }
        );
        assert_eq!(r.selection(), None);
    }

    #[test]
    fn a_click_selects_one_line_and_a_drag_carries_it_either_way() {
        let mut r = review_of(
            Some(&numbered(5)),
            "line 1\nline 2 changed\nline 3 changed\nline 4\nline 5\n",
        );
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 13);
        r.handle_mouse(MouseEventKind::Up(MouseButton::Left), 40, 13);
        assert_eq!(
            r.selection(),
            Some((3, 3)),
            "a click is a one-line selection"
        );

        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 14);
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 12);
        assert_eq!(
            r.selection(),
            Some((2, 4)),
            "dragging upward selects the rows between"
        );
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 2);
        assert_eq!(
            r.selection(),
            Some((0, 4)),
            "past the pane's top it stops at the first row shown"
        );
        r.handle_mouse(MouseEventKind::Up(MouseButton::Left), 40, 2);
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 16);
        assert_eq!(
            r.selection(),
            Some((0, 4)),
            "a move after the release does not drag"
        );
    }

    #[test]
    fn a_click_off_the_diff_selects_nothing_and_the_rows_follow_the_scroll() {
        let mut r = review_of(
            Some(&numbered(5)),
            "line 1\nline 2 changed\nline 3 changed\nline 4\nline 5\n",
        );
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 5, 12);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 9);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 29);
        assert_eq!(
            r.selection(),
            None,
            "the tree, the header and below the last row are not lines"
        );
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
        assert_eq!(r.selection_label().unwrap().range, "15");
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 10);
        assert!(
            !matches!(r.file().rows()[0], DiffRow::Fold { .. }),
            "the fold above opened"
        );
        assert_eq!(
            r.selection_label().unwrap().range,
            "15",
            "the selection did not move onto line 4"
        );
        assert_eq!(
            r.selection(),
            Some((15, 15)),
            "its drawn row moved down with the lines it covers"
        );
        assert_eq!(
            r.pane, None,
            "the pane drawn before the fold opened no longer says what is under a click"
        );
    }

    #[test]
    fn a_drag_that_ends_on_a_fold_takes_every_line_it_hides() {
        let mut r = one_change_in_thirty();
        pane_at(&mut r, 0);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 14);
        r.handle_mouse(MouseEventKind::Drag(MouseButton::Left), 40, 15);
        assert_eq!(r.selection(), Some((4, 5)));
        assert_eq!(r.selection_label(), label("15 lines", "x.rs", "16–30"));
    }

    #[test]
    fn space_opens_every_fold_and_opening_one_keeps_the_selection() {
        let mut r = one_change_in_thirty();
        r.handle_key(KeyCode::Char(' '), KeyModifiers::NONE, "");
        assert!(!r.file().has_folds(), "Space with nothing selected");

        let mut r = one_change_in_thirty();
        r.select(3, 3);
        r.file_mut().expand_all();
        r.rows_changed();
        assert_eq!(
            r.file().rows().len(),
            31,
            "thirty lines and the one removed"
        );
        assert_eq!(r.selection_label().unwrap().range, "15");
    }

    #[test]
    fn shift_and_the_arrows_select_from_the_keyboard() {
        let mut r = review_of(None, &numbered(40));
        pane_at(&mut r, 0);
        r.handle_key(KeyCode::PageDown, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        assert_eq!(
            r.selection_label(),
            label("1 line", "x.rs", "11"),
            "the first line shown"
        );
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        assert_eq!(
            r.selection_label(),
            label("1 line", "x.rs", "13"),
            "the arrows move a selection"
        );
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        assert_eq!(
            r.selection_label(),
            label("3 lines", "x.rs", "13–15"),
            "Shift extends it"
        );
        r.handle_key(KeyCode::Up, KeyModifiers::SHIFT, "");
        assert_eq!(
            r.selection_label(),
            label("2 lines", "x.rs", "13–14"),
            "and takes it back"
        );
        for _ in 0..30 {
            r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        }
        assert_eq!(
            r.selection_label().unwrap().range,
            "40",
            "it stops at the last line"
        );
        assert_eq!(r.scroll, 40 - 20, "and the pane follows it");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Up, KeyModifiers::NONE, "");
        assert_eq!(
            (r.selection(), r.scroll),
            (None, 19),
            "with nothing selected the arrows scroll again"
        );
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
            Changeset {
                files: vec![
                    ChangedFile {
                        path: "a.rs".into(),
                        before: None,
                        after: "a\n".into(),
                    },
                    ChangedFile {
                        path: "b.rs".into(),
                        before: None,
                        after: "b\n".into(),
                    },
                ],
            },
        )
        .expect("a changeset with files");
        pane_at(&mut r, 0);
        r.select(0, 0);
        r.handle_key(KeyCode::Tab, KeyModifiers::NONE, "");
        assert_eq!((r.current, r.selection(), r.pane), (1, None, None));
    }

    /// Regression: the field waited for its own `↩`, so a click showed only
    /// the `▎` edge.
    #[test]
    fn a_selection_opens_a_comment_on_the_new_file_lines() {
        let before = numbered(5);
        let after = "line 1\nline 2 changed\nline 3 changed\nline 4\nline 5\n";
        let mut r = review_of(Some(&before), after);
        // Rows: 1 ctx, del 2, del 3, add 2, add 3, 4 ctx, 5 ctx (no folds: runs of 1 and 2).
        r.select(4, 3);
        assert_eq!(r.selection(), Some((3, 4)));
        assert_eq!(r.selection_label(), label("2 lines", "x.rs", "2–3"));

        assert!(r.commenting());
        for c in "Use config".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        assert_eq!(
            r.file().comments,
            vec![PendingComment {
                lines: (2, 3),
                text: "Use config".into()
            }]
        );
        assert_eq!(r.selection(), None);
        assert_eq!(r.comment_count(), 1);
    }

    #[test]
    fn approve_waits_for_every_file_to_be_read() {
        let mut r = Review::open(
            "r1".into(),
            Changeset {
                files: vec![
                    ChangedFile {
                        path: "a.rs".into(),
                        before: Some("x\n".into()),
                        after: "y\n".into(),
                    },
                    ChangedFile {
                        path: "b.rs".into(),
                        before: None,
                        after: "z\n".into(),
                    },
                ],
            },
        )
        .expect("a changeset with files");
        assert_eq!(
            r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, ""),
            ReviewOutcome::Stay,
            "grey until every file is read"
        );
        r.mark_read();
        r.handle_key(KeyCode::Tab, KeyModifiers::NONE, "");
        assert_eq!(r.current, 1);
        r.mark_read();
        assert!(r.all_read());
        assert_eq!(
            r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, ""),
            ReviewOutcome::Decide(ReviewDecision::Approve)
        );
    }

    #[test]
    fn a_bare_enter_approves_where_ctrl_enter_cannot_be_told_apart() {
        let mut r = review_of(Some("x\n"), "y\n");
        assert_eq!(
            r.handle_key(KeyCode::Enter, KeyModifiers::NONE, ""),
            ReviewOutcome::Stay,
            "not before every file is read"
        );
        r.mark_read();
        assert_eq!(
            r.handle_key(KeyCode::Enter, KeyModifiers::NONE, ""),
            ReviewOutcome::Decide(ReviewDecision::Approve)
        );
    }

    #[test]
    fn comments_send_before_an_approve_and_a_typed_message_is_a_general_comment() {
        let mut r = review_of(Some("x\n"), "y\n");
        r.mark_read();
        r.select(0, 1);
        for c in "no".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        let ReviewOutcome::Decide(ReviewDecision::Comment { comments }) =
            r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, "and rename it")
        else {
            panic!("comments must send before an approve")
        };
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].path, "src/x.rs");
        assert_eq!(
            comments[1],
            ReviewComment {
                path: String::new(),
                lines: (0, 0),
                text: "and rename it".into()
            }
        );
    }

    #[test]
    fn escape_clears_a_selection_then_asks_before_discarding() {
        let mut r = review_of(Some("x\n"), "y\n");
        r.select(0, 0);
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert_eq!(r.selection(), None);
        assert!(r.confirm.is_none());
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert!(r.confirm.is_some());
        r.handle_key(KeyCode::Char('1'), KeyModifiers::NONE, "");
        assert!(r.confirm.is_none(), "1 keeps reviewing");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert_eq!(
            r.handle_key(KeyCode::Char('2'), KeyModifiers::NONE, ""),
            ReviewOutcome::Decide(ReviewDecision::Discard)
        );
    }

    /// Regression: the discard question ignored the arrows and `↩`.
    #[test]
    fn the_discard_question_is_a_list_like_any_other() {
        let mut r = review_of(Some("x\n"), "y\n");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        assert_eq!(
            r.handle_key(KeyCode::Enter, KeyModifiers::NONE, ""),
            ReviewOutcome::Decide(ReviewDecision::Discard)
        );
        let mut r = review_of(Some("x\n"), "y\n");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert!(r.confirm.is_none(), "esc closes it, keeping the review");
    }

    /// Regression: `esc` in the comment field discarded the text.
    #[test]
    fn escape_leaves_the_comment_field_and_the_words_survive() {
        let mut r = review_of(Some("x\n"), "y\n");
        r.select(1, 1);
        for c in "use config".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        assert_eq!(r.comment.text(), "use config", "Space is typed, not a key");
        r.handle_key(KeyCode::Esc, KeyModifiers::NONE, "");
        assert!(!r.commenting());
        r.select(1, 1);
        assert_eq!(r.comment.text(), "use config");
    }

    /// `⌃↩` sends the open field's text rather than leaving it behind.
    #[test]
    fn the_comment_field_leaves_the_selection_keys_working() {
        let mut r = review_of(None, &numbered(40));
        pane_at(&mut r, 0);
        r.select(2, 2);
        r.handle_key(KeyCode::Down, KeyModifiers::SHIFT, "");
        assert_eq!(r.selection(), Some((2, 3)));
        for c in "why".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        assert_eq!(
            r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, ""),
            ReviewOutcome::Decide(ReviewDecision::Comment {
                comments: vec![ReviewComment {
                    path: "src/x.rs".into(),
                    lines: (3, 4),
                    text: "why".into(),
                }]
            })
        );
    }

    /// Regression: `⌃↩` before every file was read did nothing visible.
    #[test]
    fn a_refused_approve_brings_up_the_first_unread_file() {
        let mut r = Review::open(
            "r".into(),
            Changeset {
                files: ["a.rs", "b.rs", "c.rs"]
                    .into_iter()
                    .map(|p| ChangedFile {
                        path: p.into(),
                        before: None,
                        after: "x\n".into(),
                    })
                    .collect(),
            },
        )
        .expect("a changeset with files");
        r.mark_read();
        assert_eq!(
            r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, ""),
            ReviewOutcome::Stay
        );
        assert_eq!(r.current, 1, "the first file not yet read");
        assert_eq!(r.unread(), 2);
    }

    /// Regression: a typed line went out as a change request while the
    /// action read "Approve".
    #[test]
    fn a_typed_line_is_sent_as_a_comment_rather_than_approved() {
        let mut r = review_of(Some("x\n"), "y\n");
        r.mark_read();
        assert!(matches!(
            r.handle_key(KeyCode::Enter, KeyModifiers::CONTROL, "rename it"),
            ReviewOutcome::Decide(ReviewDecision::Comment { .. })
        ));
    }

    /// Three files at `a.rs`, `src/b.rs`, `src/c.rs`.
    fn three_files() -> Review {
        Review::open(
            "r".into(),
            Changeset {
                files: ["a.rs", "src/b.rs", "src/c.rs"]
                    .into_iter()
                    .map(|p| ChangedFile {
                        path: p.into(),
                        before: None,
                        after: "x\n".into(),
                    })
                    .collect(),
            },
        )
        .expect("a changeset with files")
    }

    /// The tree as `ui::review::draw_tree` lays it out at screen (0, 4):
    /// blank, dots, blank, `a.rs`, `src`, `b.rs`, `c.rs`.
    fn tree_at(r: &mut Review) {
        r.tree = Some(Tree {
            x: 0,
            y: 4,
            width: 28,
            files: vec![None, None, None, Some(0), None, Some(1), Some(2)],
        });
    }

    /// Regression: the tree's rows did not answer a click.
    #[test]
    fn a_click_on_a_tree_row_shows_that_file() {
        let mut r = three_files();
        tree_at(&mut r);
        r.scroll = 3;
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 10, 10);
        assert_eq!((r.current, r.scroll), (2, 0), "c.rs, from its top");
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 10, 9);
        assert_eq!(r.current, 1, "b.rs");
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 10, 5);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 10, 8);
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 10, 20);
        assert_eq!(
            r.current, 1,
            "the dots, a folder and below the tree are not files"
        );
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 7);
        assert_eq!(r.current, 1, "past the tree's width is the diff");
    }

    #[test]
    fn sent_comments_leave_the_review_open_with_nothing_to_decide() {
        let mut r = review_of(None, &numbered(40));
        pane_at(&mut r, 0);
        r.select(2, 2);
        for c in "why".chars() {
            r.handle_key(KeyCode::Char(c), KeyModifiers::NONE, "");
        }
        r.handle_key(KeyCode::Enter, KeyModifiers::NONE, "");
        r.select(5, 5);
        r.await_agent();
        assert!(r.waiting());
        assert_eq!(r.selection(), None, "the selection goes");
        assert_eq!(r.comment_count(), 1, "the sent comment stays drawn");
        r.mark_read();
        for (code, modifiers) in [
            (KeyCode::Enter, KeyModifiers::CONTROL),
            (KeyCode::Enter, KeyModifiers::NONE),
            (KeyCode::Esc, KeyModifiers::NONE),
            (KeyCode::Down, KeyModifiers::SHIFT),
        ] {
            assert_eq!(r.handle_key(code, modifiers, "more"), ReviewOutcome::Stay);
        }
        assert!(r.confirm.is_none(), "esc asks no discard question");
        assert_eq!(r.selection(), None, "Shift ↓ selects nothing");
        r.handle_mouse(MouseEventKind::Down(MouseButton::Left), 40, 12);
        assert_eq!(r.selection(), None, "a click selects nothing");
        r.handle_mouse(MouseEventKind::ScrollDown, 40, 12);
        r.handle_key(KeyCode::Down, KeyModifiers::NONE, "");
        assert_eq!(
            r.scroll,
            WHEEL_ROWS + 1,
            "the wheel and the arrows still scroll"
        );
    }

    #[test]
    fn the_next_changeset_replaces_a_waiting_review_and_keeps_what_was_read() {
        let mut previous = three_files();
        previous.mark_read();
        previous.go_to_file(2);
        previous.mark_read();
        previous.scroll = 3;
        previous.keys_shown = true;
        previous.title = "Add a limit".into();
        previous.await_agent();

        let mut next = Review::open(
            "r2".into(),
            Changeset {
                files: vec![
                    ChangedFile {
                        path: "a.rs".into(),
                        before: None,
                        after: "x\n".into(),
                    },
                    ChangedFile {
                        path: "src/b.rs".into(),
                        before: None,
                        after: "x\n".into(),
                    },
                    ChangedFile {
                        path: "src/c.rs".into(),
                        before: None,
                        after: "y\n".into(),
                    },
                ],
            },
        )
        .expect("a changeset with files");
        next.carry_from(&previous);
        assert!(!next.waiting());
        let read: Vec<bool> = next.files().iter().map(|f| f.read).collect();
        assert_eq!(
            read,
            vec![true, false, false],
            "a.rs is unchanged and was read; b.rs was never read; c.rs changed"
        );
        assert_eq!(
            (next.current, next.scroll),
            (2, 0),
            "c.rs stays on screen, its diff new"
        );
        assert!(next.keys_shown);
        assert_eq!(next.title, "Add a limit");
        assert_eq!(
            next.comment_count(),
            0,
            "the sent comments are the agent's now"
        );
    }

    #[test]
    fn each_drawn_row_and_the_lines_it_stands_for_agree_through_every_fold() {
        let before = numbered(200);
        let after = before
            .replace("line 20\n", "line twenty\n")
            .replace("line 150\n", "line 150!\n");
        let mut r = review_of(Some(&before), &after);
        let agree = |file: &ReviewFile| {
            for row in 0..file.row_count() {
                let (first, last) = file.span_of(row).unwrap();
                assert_eq!((file.row_of(first), file.row_of(last)), (row, row));
            }
            assert_eq!(file.row_of(usize::MAX), 0, "past the end");
        };
        agree(r.file());
        let fold = r.file().drawn.iter().find(|span| span.2).unwrap().0;
        r.file_mut().expand(fold);
        agree(r.file());
        r.file_mut().expand_all();
        agree(r.file());
        assert_eq!(r.file().row_count(), r.file().unfolded.len());
    }

    /// Regression: past a 2^20-cell table the diff gave up and showed every
    /// line between the first and last change as removed and added.
    #[test]
    fn two_edits_far_apart_in_a_long_file_are_two_changed_lines() {
        let before = numbered(20_000);
        let after = before
            .replace("line 10\n", "line ten\n")
            .replace("line 19990\n", "line 19,990\n");
        let r = review_of(Some(&before), &after);
        assert_eq!((r.file().added_lines, r.file().removed_lines), (2, 2));
        assert!(
            r.file().row_count() < 30,
            "the unchanged lines fold: {} rows",
            r.file().row_count()
        );
    }

    #[test]
    fn a_rewrite_is_its_old_lines_out_and_new_ones_in() {
        let before = numbered(1100);
        let after: String = (1..=1100).map(|i| format!("row {i}\n")).collect();
        let r = review_of(Some(&before), &after);
        let rows = r.file().rows();
        assert!(matches!(rows[0], DiffRow::Del { .. }));
        assert!(matches!(rows[1100], DiffRow::Add { .. }));
        assert_eq!((r.file().added_lines, r.file().removed_lines), (1100, 1100));
    }
}
