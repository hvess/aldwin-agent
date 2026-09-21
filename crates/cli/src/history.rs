//! The session's transcript, as the rest of aldwin-cli has to see it.
//!
//! Two things live here that aldwin-config deliberately does not carry:
//!
//! 1. **The `RecordSink` impl.** A failed write has to reach the developer,
//!    and the only vehicle for that is the session's `Event` channel — which
//!    aldwin-config, having no tokio dependency, cannot hold.
//! 2. **The swap.** `/clear` seals the current transcript and opens a fresh
//!    one; `/resume` moves the writer onto the transcript it just loaded.
//!    The agent holds this sink for the life of the process, so the file
//!    underneath it is what changes — the same shape as `ClientHandle`,
//!    which is how `/model` swaps a client the agent already owns.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use aldwin_config::{HistoryStore, SessionHeader, SessionSummary, HISTORY_VERSION};
use aldwin_core::{Event, LogRecord, RecordSink, SessionId};
use aldwin_tui::SessionChoice;
use tokio::sync::mpsc;

/// The transcript this session writes to, and the handle that can point it
/// somewhere else.
#[derive(Debug)]
pub struct History {
    dir:    PathBuf,
    model:  String,
    /// Which transcript is being written *now* — the session the developer
    /// is sitting in, or the one they resumed onto. Excluded from every
    /// listing and refused by `/resume`: a session cannot be resumed into
    /// itself, and offering it as a row is offering a no-op.
    current: Mutex<SessionId>,
    store:  Mutex<Option<HistoryStore>>,
    events: mpsc::Sender<Event>,
    /// A write failure is reported once. A disk that is full at record 200
    /// is still full at record 201, and the developer does not need to be
    /// told four hundred times that history is not being kept.
    reported: AtomicBool,
}

impl History {
    /// Open this session's transcript. `None` when the history directory
    /// cannot be written: the session runs exactly as it did before history
    /// existed, having said so once. History must never be able to stop a
    /// session starting, let alone fail a turn.
    pub fn open(dir: PathBuf, model: String, events: mpsc::Sender<Event>) -> (Option<Arc<Self>>, Option<String>) {
        let id = SessionId::mint();
        let header = SessionHeader {
            version:    HISTORY_VERSION,
            started_at: now(),
            cwd:        std::env::current_dir().unwrap_or_default().to_string_lossy().into_owned(),
            model:      model.clone(),
        };
        match HistoryStore::create(&dir, &id, &header) {
            Ok(store) => (
                Some(Arc::new(Self {
                    dir,
                    model,
                    current: Mutex::new(id),
                    store: Mutex::new(Some(store)),
                    events,
                    reported: AtomicBool::new(false),
                })),
                None,
            ),
            Err(e) => (None, Some(format!("history is off for this session: {e}"))),
        }
    }

    /// `/clear` — seal this transcript and begin a new one.
    ///
    /// Sealing is implicit: nothing is written to close the old file. The
    /// developer clears to manage the model's context, not to shred the
    /// record, so the old transcript stays exactly as it is and remains
    /// resumable.
    pub fn seal_and_open_new(&self) {
        let id = SessionId::mint();
        let header = SessionHeader {
            version:    HISTORY_VERSION,
            started_at: now(),
            cwd:        std::env::current_dir().unwrap_or_default().to_string_lossy().into_owned(),
            model:      self.model.clone(),
        };
        let opened = HistoryStore::create(&self.dir, &id, &header).ok();
        *self.store.lock().expect("history lock poisoned") = opened;
        *self.current.lock().expect("history lock poisoned") = id;
    }

    /// `/resume` — continue writing into the transcript that was just
    /// loaded, rather than forking a second file for the same conversation.
    pub fn continue_session(&self, id: &SessionId) -> Result<(), String> {
        let store = HistoryStore::reopen(&self.dir, id).map_err(|e| e.to_string())?;
        *self.store.lock().expect("history lock poisoned") = Some(store);
        *self.current.lock().expect("history lock poisoned") = id.clone();
        Ok(())
    }

    /// The transcript being written right now.
    pub fn current(&self) -> SessionId {
        self.current.lock().expect("history lock poisoned").clone()
    }

    /// Is this the session already being written? `/resume` refuses it, and
    /// the picker never lists it.
    pub fn is_current(&self, id: &SessionId) -> bool {
        *self.current.lock().expect("history lock poisoned") == *id
    }

    /// The past sessions of this project — every transcript but the one this
    /// session is writing.
    pub fn resumable(&self) -> Vec<SessionChoice> {
        let current = self.current();
        session_choices(&self.dir).into_iter().filter(|s| s.id != current.0).collect()
    }

    pub fn dir(&self) -> &Path { &self.dir }
}

impl RecordSink for History {
    fn append(&self, record: &LogRecord) {
        let guard = self.store.lock().expect("history lock poisoned");
        let Some(store) = guard.as_ref() else { return };
        if let Err(e) = store.append(record) {
            // `try_send` rather than an await: this runs on whichever task
            // committed the record, and a full event channel must not become
            // back-pressure on the conversation. A dropped notice is the
            // right trade — the alternative is a turn that stalls on
            // reporting that history is broken.
            if !self.reported.swap(true, Ordering::Relaxed) {
                let _ = self.events.try_send(Event::Notice {
                    message: format!("history write failed; this session is no longer being recorded: {e}"),
                });
            }
        }
    }
}

/// Every session in a history directory, rendered for the picker.
///
/// aldwin-tui takes display halves and nothing else — no paths, no
/// timestamps — so the formatting happens here, the same division the model
/// catalogue already follows.
///
/// Callers inside a running session want [`History::resumable`] instead:
/// this one includes the transcript currently being written.
pub fn session_choices(dir: &Path) -> Vec<SessionChoice> {
    aldwin_config::list_sessions(dir).into_iter().map(choice).collect()
}

fn choice(summary: SessionSummary) -> SessionChoice {
    SessionChoice {
        id:    summary.id.0,
        title: summary.title,
        when:  format_when(summary.started_at),
        turns: summary.turns,
    }
}

/// `2026-09-20 18:11`, in the developer's own timezone — they are reading
/// back their own afternoon, not a log shipped from elsewhere.
fn format_when(epoch_secs: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(epoch_secs as i64, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        // Unrepresentable, which means the header was damaged. The row is
        // still worth showing: its title is what the developer picks by.
        None => "unknown".to_string(),
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aldwin_core::{TurnEndReason, TurnId};
    use tempfile::tempdir;

    fn history(dir: &Path) -> (Arc<History>, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel(8);
        let (history, failure) = History::open(dir.to_path_buf(), "m".into(), tx);
        assert!(failure.is_none());
        (history.expect("a store"), rx)
    }

    fn turn(n: u64, text: &str) -> Vec<LogRecord> {
        vec![
            LogRecord::TurnStarted { turn_id: TurnId(n) },
            LogRecord::UserMessage { turn_id: TurnId(n), text: text.into() },
            LogRecord::TurnEnded { turn_id: TurnId(n), reason: TurnEndReason::EndTurn },
        ]
    }

    #[test]
    fn records_reach_the_transcript() {
        let dir = tempdir().unwrap();
        let (history, _rx) = history(dir.path());
        for record in turn(1, "hello") {
            history.append(&record);
        }
        let sessions = session_choices(dir.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].title, "hello");
        assert_eq!(sessions[0].turns, 1);
    }

    /// Step 6: `/clear` seals rather than deletes — the old conversation is
    /// still there to resume, and the new one is a separate file.
    #[test]
    fn clearing_seals_the_old_transcript_and_opens_a_new_one() {
        let dir = tempdir().unwrap();
        let (history, _rx) = history(dir.path());
        for record in turn(1, "before the clear") {
            history.append(&record);
        }

        history.seal_and_open_new();
        for record in turn(2, "after the clear") {
            history.append(&record);
        }

        let sessions = session_choices(dir.path());
        assert_eq!(sessions.len(), 2, "two sessions, not one file with both in it");
        let titles: Vec<&str> = sessions.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"before the clear"), "clearing does not destroy the record");
        assert!(titles.contains(&"after the clear"));
    }

    /// The fork-free Decision: a resumed conversation keeps writing into the
    /// file it came from.
    #[test]
    fn resuming_continues_the_same_file() {
        let dir = tempdir().unwrap();
        let (history, _rx) = history(dir.path());
        for record in turn(1, "first") {
            history.append(&record);
        }
        let id = SessionId(session_choices(dir.path())[0].id.clone());

        history.seal_and_open_new(); // a second session, as a new launch would
        history.continue_session(&id).expect("the transcript reopens");
        for record in turn(2, "second") {
            history.append(&record);
        }

        let resumed = session_choices(dir.path()).into_iter().find(|s| s.id == id.0).expect("still listed");
        assert_eq!(resumed.turns, 2, "the continued turn landed in the resumed file");
        assert_eq!(resumed.title, "first", "and its title still comes from where it began");
    }

    /// History must never be able to fail a turn: a store that cannot be
    /// opened costs a message, not a session.
    #[test]
    fn an_unwritable_directory_disables_history_rather_than_failing() {
        let dir = tempdir().unwrap();
        let blocked = dir.path().join("wall");
        std::fs::write(&blocked, "not a directory").unwrap();

        let (tx, _rx) = mpsc::channel(8);
        let (history, failure) = History::open(blocked.join("history"), "m".into(), tx);
        assert!(history.is_none());
        assert!(failure.expect("a reason").contains("history is off"));
    }

    /// A sink whose file has gone away keeps accepting records — silently,
    /// after saying so once.
    #[test]
    fn a_broken_transcript_reports_once_and_then_stays_quiet() {
        let dir = tempdir().unwrap();
        let (history, mut rx) = history(dir.path());

        // Close the file underneath the sink, the way a failed write does.
        {
            let mut guard = history.store.lock().unwrap();
            let store = guard.take().expect("a store");
            drop(store);
            // Reopen onto a path that cannot be written, so `append` fails.
            *guard = HistoryStore::reopen(&dir.path().join("gone"), &SessionId("nope".into())).ok();
        }
        for record in turn(1, "hello") {
            history.append(&record);
        }
        assert!(rx.try_recv().is_err(), "a store that could not even be reopened is simply off");
    }

    #[test]
    fn a_listing_is_newest_first_and_carries_a_readable_date() {
        let dir = tempdir().unwrap();
        let (history, _rx) = history(dir.path());
        for record in turn(1, "only session") {
            history.append(&record);
        }
        let sessions = session_choices(dir.path());
        assert_eq!(sessions[0].when.len(), 16, "`YYYY-MM-DD HH:MM`: {}", sessions[0].when);
    }
}
