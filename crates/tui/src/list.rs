//! The one list control every question is asked with: numbered rows, `↑↓`
//! to choose, `↩` or the number to pick, `esc` to close. The design draws a
//! `QuestionPanel` and a `CommandRow` list; a provider or a session picker
//! is the same control with different rows, so there is one of these and
//! not four.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

/// One row: what it says, and — for a command list — its purpose in the
/// second column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    pub label: String,
    pub detail: String,
}

impl ListRow {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: String::new(),
        }
    }

    pub fn with_detail(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// What one keypress did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOutcome {
    Stay,
    Close,
    Chose(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    pub rows: Vec<ListRow>,
    pub selected: usize,
}

impl List {
    pub fn new(rows: Vec<ListRow>) -> Self {
        Self { rows, selected: 0 }
    }

    /// Opens with `selected` on the row that is current already — a
    /// picker over the session's own model, say — so `↩` confirms rather
    /// than moves.
    pub fn opened_on(mut self, index: usize) -> Self {
        self.selected = index.min(self.rows.len().saturating_sub(1));
        self
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> ListOutcome {
        if self.rows.is_empty() {
            return ListOutcome::Close;
        }
        let last = self.rows.len() - 1;
        match (code, modifiers) {
            (KeyCode::Up, _) => self.selected = self.selected.saturating_sub(1),
            (KeyCode::Down, _) => self.selected = (self.selected + 1).min(last),
            (KeyCode::Enter, _) => return ListOutcome::Chose(self.selected),
            (KeyCode::Esc, _) => return ListOutcome::Close,
            (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                return ListOutcome::Close
            }
            // Press the number: the design's own instruction.
            (KeyCode::Char(c), _) if c.is_ascii_digit() && c != '0' => {
                let idx = (c as u8 - b'1') as usize;
                if idx <= last {
                    return ListOutcome::Chose(idx);
                }
            }
            _ => {}
        }
        ListOutcome::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> List {
        List::new(vec![
            ListRow::new("Yes"),
            ListRow::new("No"),
            ListRow::new("Chat about this"),
        ])
    }

    #[test]
    fn arrows_clamp_and_enter_picks() {
        let mut l = list();
        assert_eq!(
            l.handle_key(KeyCode::Up, KeyModifiers::NONE),
            ListOutcome::Stay
        );
        assert_eq!(l.selected, 0);
        l.handle_key(KeyCode::Down, KeyModifiers::NONE);
        l.handle_key(KeyCode::Down, KeyModifiers::NONE);
        l.handle_key(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(l.selected, 2);
        assert_eq!(
            l.handle_key(KeyCode::Enter, KeyModifiers::NONE),
            ListOutcome::Chose(2)
        );
    }

    #[test]
    fn a_number_picks_directly_and_an_out_of_range_one_does_nothing() {
        let mut l = list();
        assert_eq!(
            l.handle_key(KeyCode::Char('2'), KeyModifiers::NONE),
            ListOutcome::Chose(1)
        );
        assert_eq!(
            l.handle_key(KeyCode::Char('9'), KeyModifiers::NONE),
            ListOutcome::Stay
        );
        assert_eq!(
            l.handle_key(KeyCode::Char('0'), KeyModifiers::NONE),
            ListOutcome::Stay
        );
    }

    #[test]
    fn escape_and_ctrl_c_close() {
        let mut l = list();
        assert_eq!(
            l.handle_key(KeyCode::Esc, KeyModifiers::NONE),
            ListOutcome::Close
        );
        assert_eq!(
            l.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            ListOutcome::Close
        );
    }

    #[test]
    fn opened_on_clamps_to_the_rows() {
        assert_eq!(list().opened_on(7).selected, 2);
        assert_eq!(List::new(vec![]).opened_on(3).selected, 0);
    }
}
