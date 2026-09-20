//! The session picker — what bare `/resume` opens.
//!
//! One list where [`crate::picker`] has two, and the same control otherwise:
//! it answers by composing `/resume <id>` and submitting it exactly as if
//! the developer had typed it. mjolnir-cli's interceptor stays the one place
//! that knows what resuming *does* — this screen owns only how the question
//! is asked. Same rule, same reason, as the model picker's own doc comment.
//!
//! The rows arrive as display halves from mjolnir-cli's bootstrap
//! ([`SessionChoice`]), never as paths: mjolnir-tui reads no files, which is
//! why the transcript directory is not mentioned anywhere in this crate.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use crate::picker::{PickerOutcome, PickerRow};

/// One past session, as the list needs to show it. Already rendered for
/// display by the caller — this crate never sees a timestamp or a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionChoice {
    /// What `/resume <id>` is composed with.
    pub id:    String,
    /// First user message, one line — what the developer will recognise the
    /// session by.
    pub title: String,
    /// When it started, already formatted (`2026-09-20 18:11`).
    pub when:  String,
    pub turns: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumePicker {
    pub sessions: Vec<SessionChoice>,
    pub selected: usize,
}

impl ResumePicker {
    /// `None` when there is nothing to resume — bare `/resume` then falls
    /// through to the interceptor, which says so in a Notice rather than
    /// opening an empty panel. An empty list is an answer, and a panel with
    /// no rows is not a way to give it.
    pub fn open(sessions: Vec<SessionChoice>) -> Option<Self> {
        if sessions.is_empty() {
            return None;
        }
        Some(Self { sessions, selected: 0 })
    }

    /// Newest first, so the row the developer most likely wants is the one
    /// the cursor already sits on — the list's equivalent of the model
    /// picker opening on the row in use.
    pub fn rows(&self) -> Vec<PickerRow> {
        self.sessions
            .iter()
            .map(|s| PickerRow {
                label:   s.title.clone(),
                detail:  format!("{} · {}", s.when, turns_label(s.turns)),
                // Nothing here is "current": a session you are resuming is
                // by definition not the one you are in.
                current: false,
            })
            .collect()
    }

    pub fn selected(&self) -> usize { self.selected }

    fn len(&self) -> usize { self.sessions.len() }

    /// One key. `Esc` closes outright — unlike the model picker there is no
    /// stage above this one to step back to.
    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> ResumeOutcome {
        if matches!(code, KeyCode::Char('c') | KeyCode::Char('d')) && modifiers.contains(KeyModifiers::CONTROL) {
            return ResumeOutcome::Close;
        }
        match code {
            KeyCode::Esc | KeyCode::Left => ResumeOutcome::Close,
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                ResumeOutcome::Stay
            }
            KeyCode::Down => {
                // Clamped, never wrapping — the rule every list in this crate
                // follows.
                self.selected = (self.selected + 1).min(self.len().saturating_sub(1));
                ResumeOutcome::Stay
            }
            KeyCode::Right | KeyCode::Enter => match self.sessions.get(self.selected) {
                Some(session) => ResumeOutcome::Chosen { id: session.id.clone() },
                None => ResumeOutcome::Stay,
            },
            // The decision panel's direct-pick accelerator, on the same
            // 1-based numbering the rows are drawn with.
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = (c as u8 - b'1') as usize;
                if index < self.len() {
                    self.selected = index;
                }
                ResumeOutcome::Stay
            }
            _ => ResumeOutcome::Stay,
        }
    }
}

/// What one keypress did. Shaped like [`PickerOutcome`] and converted into
/// one at the edge, so `App` handles both pickers' closing and committing
/// through a single path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    Stay,
    Close,
    Chosen { id: String },
}

impl ResumeOutcome {
    pub fn into_picker_outcome(self) -> PickerOutcome {
        match self {
            Self::Stay => PickerOutcome::Stay,
            Self::Close => PickerOutcome::Close,
            // `provider` carries the id and `model` is unused — the two
            // pickers compose different commands and only `App` does the
            // composing.
            Self::Chosen { id } => PickerOutcome::Chosen { provider: id, model: String::new() },
        }
    }
}

/// "1 turn" / "4 turns" — the panel writes lowercase sentence-case prose,
/// and "1 turns" is the kind of thing a reader notices instead of the row.
fn turns_label(turns: usize) -> String {
    if turns == 1 { "1 turn".to_string() } else { format!("{turns} turns") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sessions() -> Vec<SessionChoice> {
        vec![
            SessionChoice { id: "s3".into(), title: "newest".into(), when: "2026-09-20 18:11".into(), turns: 1 },
            SessionChoice { id: "s2".into(), title: "middle".into(), when: "2026-09-19 09:02".into(), turns: 4 },
            SessionChoice { id: "s1".into(), title: "oldest".into(), when: "2026-09-18 14:30".into(), turns: 12 },
        ]
    }

    fn picker() -> ResumePicker {
        ResumePicker::open(sessions()).expect("a list")
    }

    fn key(picker: &mut ResumePicker, code: KeyCode) -> ResumeOutcome {
        picker.handle_key(code, KeyModifiers::NONE)
    }

    #[test]
    fn an_empty_history_opens_nothing() {
        assert!(ResumePicker::open(Vec::new()).is_none());
    }

    #[test]
    fn it_opens_on_the_newest_session() {
        assert_eq!(picker().selected(), 0);
    }

    #[test]
    fn enter_answers_with_the_selected_session() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Down);
        assert_eq!(key(&mut picker, KeyCode::Enter), ResumeOutcome::Chosen { id: "s2".into() });
    }

    #[test]
    fn a_digit_picks_the_row_it_numbers() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Char('3'));
        assert_eq!(picker.selected(), 2);
        assert_eq!(key(&mut picker, KeyCode::Enter), ResumeOutcome::Chosen { id: "s1".into() });
    }

    #[test]
    fn a_digit_past_the_end_moves_nothing() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Char('9'));
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn movement_clamps_rather_than_wrapping() {
        let mut picker = picker();
        key(&mut picker, KeyCode::Up);
        assert_eq!(picker.selected(), 0, "up from the top stays");
        for _ in 0..10 {
            key(&mut picker, KeyCode::Down);
        }
        assert_eq!(picker.selected(), 2, "down past the end stays on the last row");
    }

    #[test]
    fn esc_closes_outright() {
        assert_eq!(key(&mut picker(), KeyCode::Esc), ResumeOutcome::Close);
    }

    #[test]
    fn ctrl_c_closes_it_like_every_other_panel() {
        let mut picker = picker();
        assert_eq!(picker.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL), ResumeOutcome::Close);
    }

    #[test]
    fn a_row_reads_as_a_date_and_a_turn_count() {
        let rows = picker().rows();
        assert_eq!(rows[0].label, "newest");
        assert_eq!(rows[0].detail, "2026-09-20 18:11 · 1 turn");
        assert_eq!(rows[1].detail, "2026-09-19 09:02 · 4 turns");
    }
}
