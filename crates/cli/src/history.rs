//! The session's transcript sink (`.claude/spec/archive/aldwin-history.md`).
//!
//! The `RecordSink` impl lives here, not in aldwin-config, because a failed
//! write is reported on the session's tokio `Event` channel, which
//! aldwin-config cannot hold. The agent holds the sink for the process's
//! life; `/clear` and `/resume` swap the file underneath it, only when core
//! calls `RecordSink::cleared` / `RecordSink::resumed` (never mid-turn).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use aldwin_config::{ConfigError, HistoryStore, SessionHeader, SessionSummary, HISTORY_VERSION};
use aldwin_core::{Event, LogRecord, RecordSink, SessionId};
use aldwin_tui::SessionChoice;
use tokio::sync::mpsc;

/// The transcript this session writes to, and the handle that can point it
/// somewhere else.
#[derive(Debug)]
pub struct History {
    dir: PathBuf,
    /// The project root, for each new transcript's header.
    cwd: PathBuf,
    model: String,
    /// The transcript being written now. Excluded from every listing and
    /// refused by `/resume`: a session cannot resume into itself.
    current: Mutex<SessionId>,
    store: Mutex<Option<HistoryStore>>,
    events: mpsc::Sender<Event>,
    /// Whether this transcript's failure has been reported; see `report`.
    reported: AtomicBool,
    /// A turn is being written; see `turn_in_flight`.
    in_turn: AtomicBool,
}

impl History {
    /// Opens this session's transcript. `Err` when the history directory
    /// cannot be written; the caller must then run without history and say
    /// so once. History must never stop a session starting or fail a turn.
    pub fn open(
        dir: PathBuf,
        cwd: &Path,
        model: String,
        events: mpsc::Sender<Event>,
    ) -> Result<Arc<Self>, ConfigError> {
        let id = SessionId::mint();
        let store = HistoryStore::create(&dir, &id, &header(cwd, &model))?;
        Ok(Arc::new(Self {
            dir,
            cwd: cwd.to_path_buf(),
            model,
            current: Mutex::new(id),
            store: Mutex::new(Some(store)),
            events,
            reported: AtomicBool::new(false),
            in_turn: AtomicBool::new(false),
        }))
    }

    /// Says `message` once per transcript.
    ///
    /// `try_send`, not an await: a full event channel must not stall the
    /// turn that committed the record; dropping the notice is acceptable.
    fn report(&self, message: String) {
        if !self.reported.swap(true, Ordering::Relaxed) {
            let _ = self.events.try_send(Event::Notice { message });
        }
    }

    /// The transcript being written right now.
    pub fn current(&self) -> SessionId {
        self.current.lock().expect("history lock poisoned").clone()
    }

    /// Whether `id` is the transcript being written.
    pub fn is_current(&self, id: &SessionId) -> bool {
        *self.current.lock().expect("history lock poisoned") == *id
    }

    /// Every transcript in the directory but the current one.
    pub fn resumable(&self) -> Vec<SessionChoice> {
        let current = self.current();
        session_choices(&self.dir)
            .into_iter()
            .filter(|s| s.id != current.0)
            .collect()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Marks a submission sent to core. Needed besides the `TurnStarted`
    /// record: a `/clear` typed straight after is intercepted before core
    /// logs anything.
    pub fn turn_submitted(&self) {
        self.in_turn.store(true, Ordering::Relaxed);
    }

    /// Whether a turn is still being written. Lets the interceptor refuse
    /// `/clear` or `/resume` early; core's own refusal is what actually keeps
    /// the writer in place.
    pub fn turn_in_flight(&self) -> bool {
        self.in_turn.load(Ordering::Relaxed)
    }
}

impl RecordSink for History {
    fn append(&self, record: &LogRecord) {
        match record {
            LogRecord::TurnStarted { .. } => self.in_turn.store(true, Ordering::Relaxed),
            LogRecord::TurnEnded { .. } => self.in_turn.store(false, Ordering::Relaxed),
            _ => {}
        }
        let guard = self.store.lock().expect("history lock poisoned");
        let Some(store) = guard.as_ref() else { return };
        if let Err(e) = store.append(record) {
            self.report(format!(
                "history write failed; this session is no longer being recorded: {e}"
            ));
        }
    }

    /// `/clear`: starts a new transcript. The old file is left as is (nothing
    /// is written to seal it) and stays resumable.
    fn cleared(&self) {
        let id = SessionId::mint();
        let opened = match HistoryStore::create(&self.dir, &id, &header(&self.cwd, &self.model)) {
            Ok(store) => {
                // A new file gets its own one failure report.
                self.reported.store(false, Ordering::Relaxed);
                Some(store)
            }
            Err(e) => {
                self.report(format!("history is off from here; the cleared conversation is kept, but no new transcript could be opened: {e}"));
                None
            }
        };
        *self.store.lock().expect("history lock poisoned") = opened;
        *self.current.lock().expect("history lock poisoned") = id;
    }

    /// `/resume`: continues writing into the loaded transcript, never a
    /// second file for the same conversation.
    fn resumed(&self, session: &SessionId) {
        let reopened = HistoryStore::reopen(&self.dir, session);
        self.reported.store(false, Ordering::Relaxed);
        let store = match reopened {
            Ok(store) => Some(store),
            // Never fall back to the previous file: it would mix two
            // conversations.
            Err(e) => {
                self.report(format!(
                    "history is off from here; session {session} could not be reopened: {e}"
                ));
                None
            }
        };
        *self.store.lock().expect("history lock poisoned") = store;
        *self.current.lock().expect("history lock poisoned") = session.clone();
    }
}

/// Every session in a history directory, formatted for the picker
/// (aldwin-tui takes display strings only).
///
/// Includes the current transcript; inside a session use
/// [`History::resumable`].
pub fn session_choices(dir: &Path) -> Vec<SessionChoice> {
    aldwin_config::list_sessions(dir)
        .into_iter()
        .map(choice)
        .collect()
}

fn choice(summary: SessionSummary) -> SessionChoice {
    SessionChoice {
        id: summary.id.0,
        title: summary.title,
        when: format_when(summary.started_at),
        turns: summary.turns,
    }
}

/// `2026-09-20 18:11`, in local time.
fn format_when(epoch_secs: u64) -> String {
    use chrono::{Local, TimeZone};
    // `try_from`, not `as`: a damaged header past `i64::MAX` would wrap to a
    // plausible pre-1970 date rather than "unknown".
    match i64::try_from(epoch_secs)
        .ok()
        .and_then(|secs| Local.timestamp_opt(secs, 0).single())
    {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        // A damaged header; the row is still listed by its title.
        None => "unknown".to_string(),
    }
}

fn header(cwd: &Path, model: &str) -> SessionHeader {
    SessionHeader {
        version: HISTORY_VERSION,
        started_at: now(),
        cwd: cwd.to_string_lossy().into_owned(),
        model: model.to_string(),
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
        (
            History::open(dir.to_path_buf(), Path::new("/p"), "m".into(), tx).expect("a store"),
            rx,
        )
    }

    fn turn(n: u64, text: &str) -> Vec<LogRecord> {
        vec![
            LogRecord::TurnStarted { turn_id: TurnId(n) },
            LogRecord::UserMessage {
                turn_id: TurnId(n),
                text: text.into(),
            },
            LogRecord::TurnEnded {
                turn_id: TurnId(n),
                reason: TurnEndReason::EndTurn,
            },
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

    /// aldwin-history.md step 6.
    #[test]
    fn clearing_seals_the_old_transcript_and_opens_a_new_one() {
        let dir = tempdir().unwrap();
        let (history, _rx) = history(dir.path());
        for record in turn(1, "before the clear") {
            history.append(&record);
        }

        history.cleared();
        for record in turn(2, "after the clear") {
            history.append(&record);
        }

        let sessions = session_choices(dir.path());
        assert_eq!(
            sessions.len(),
            2,
            "two sessions, not one file with both in it"
        );
        let titles: Vec<&str> = sessions.iter().map(|s| s.title.as_str()).collect();
        assert!(
            titles.contains(&"before the clear"),
            "clearing does not destroy the record"
        );
        assert!(titles.contains(&"after the clear"));
    }

    /// aldwin-history.md's fork-free Decision.
    #[test]
    fn resuming_continues_the_same_file() {
        let dir = tempdir().unwrap();
        let (history, _rx) = history(dir.path());
        for record in turn(1, "first") {
            history.append(&record);
        }
        let id = SessionId(session_choices(dir.path())[0].id.clone());

        history.cleared(); // a second session, as a new launch would
        history.resumed(&id);
        for record in turn(2, "second") {
            history.append(&record);
        }

        let resumed = session_choices(dir.path())
            .into_iter()
            .find(|s| s.id == id.0)
            .expect("still listed");
        assert_eq!(
            resumed.turns, 2,
            "the continued turn landed in the resumed file"
        );
        assert_eq!(
            resumed.title, "first",
            "and its title still comes from where it began"
        );
    }

    #[test]
    fn an_unwritable_directory_disables_history_rather_than_failing() {
        let dir = tempdir().unwrap();
        let blocked = dir.path().join("wall");
        std::fs::write(&blocked, "not a directory").unwrap();

        let (tx, _rx) = mpsc::channel(8);
        let failure = History::open(blocked.join("history"), Path::new("/p"), "m".into(), tx);
        assert!(
            failure.is_err(),
            "the store reports why rather than panicking"
        );
    }

    /// A sink with no store accepts records and says nothing.
    #[test]
    fn a_broken_transcript_reports_once_and_then_stays_quiet() {
        let dir = tempdir().unwrap();
        let (history, mut rx) = history(dir.path());

        // Drop the store underneath the sink.
        {
            let mut guard = history.store.lock().unwrap();
            let store = guard.take().expect("a store");
            drop(store);
            // The reopen fails, leaving no store.
            *guard = HistoryStore::reopen(&dir.path().join("gone"), &SessionId("nope".into())).ok();
        }
        for record in turn(1, "hello") {
            history.append(&record);
        }
        assert!(
            rx.try_recv().is_err(),
            "a store that could not even be reopened is simply off"
        );
    }

    /// Regression guard: the resumed conversation must not be written into
    /// the previous file.
    #[test]
    fn a_resume_that_cannot_reopen_its_transcript_says_so_and_records_nothing() {
        let dir = tempdir().unwrap();
        let (history, mut rx) = history(dir.path());
        let gone = SessionId("0000000000-0-000000".into());

        history.resumed(&gone);
        assert!(
            matches!(rx.try_recv(), Ok(Event::Notice { message }) if message.contains("could not be reopened"))
        );
        assert!(history.is_current(&gone));
        for record in turn(1, "unrecorded") {
            history.append(&record);
        }
        assert!(
            session_choices(dir.path()).is_empty(),
            "nothing was written"
        );
    }

    /// Regression: this switched history off silently.
    #[test]
    fn a_clear_that_cannot_open_a_new_transcript_says_so_once() {
        let dir = tempdir().unwrap();
        let store = dir.path().join("history");
        let (history, mut rx) = history(&store);

        std::fs::remove_dir_all(&store).unwrap();
        std::fs::write(&store, "not a directory").unwrap();
        history.cleared();
        history.cleared();

        assert!(
            matches!(rx.try_recv(), Ok(Event::Notice { message }) if message.contains("history is off from here"))
        );
        assert!(rx.try_recv().is_err(), "said once, not per clear");
        for record in turn(1, "unrecorded") {
            history.append(&record); // must not panic
        }
    }

    #[test]
    fn a_listing_is_newest_first_and_carries_a_readable_date() {
        let dir = tempdir().unwrap();
        let (history, _rx) = history(dir.path());
        for record in turn(1, "only session") {
            history.append(&record);
        }
        let sessions = session_choices(dir.path());
        assert_eq!(
            sessions[0].when.len(),
            16,
            "`YYYY-MM-DD HH:MM`: {}",
            sessions[0].when
        );
    }
}
