//! The session's transcript, as the rest of aldwin-cli has to see it.
//!
//! Two things live here that aldwin-config deliberately does not carry:
//!
//! 1. **The `RecordSink` impl.** A failed write has to reach the developer,
//!    and the only vehicle for that is the session's `Event` channel — which
//!    aldwin-config, having no tokio dependency, cannot hold.
//! 2. **The swap.** `/clear` seals the current transcript and opens a fresh
//!    one; `/resume` moves the writer onto the transcript it loaded. Both
//!    happen when core acts on the command and tells its sink
//!    (`RecordSink::cleared`, `RecordSink::resumed`) — never on the way
//!    past, since core refuses either one while a turn runs. The agent holds
//!    this sink for the life of the process, so the file underneath it is
//!    what changes — the same shape as `ClientHandle`, which is how `/model`
//!    swaps a client the agent already owns.

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
    /// Which transcript is being written *now* — the session the developer
    /// is sitting in, or the one they resumed onto. Excluded from every
    /// listing and refused by `/resume`: a session cannot be resumed into
    /// itself, and offering it as a row is offering a no-op.
    current: Mutex<SessionId>,
    store: Mutex<Option<HistoryStore>>,
    events: mpsc::Sender<Event>,
    /// Whether this transcript's failure has been said — see `report`.
    reported: AtomicBool,
    /// A turn is being written — see `turn_in_flight`.
    in_turn: AtomicBool,
}

impl History {
    /// Open this session's transcript. `Err` when the history directory
    /// cannot be written: the session then runs without one, having said
    /// so once. History must never be able to stop a session starting, let
    /// alone fail a turn.
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

    /// Says `message` once per transcript. A disk that is full at record 200
    /// is still full at record 201, and the developer does not need to be
    /// told four hundred times that history is not being kept.
    ///
    /// `try_send` rather than an await: this runs on whichever task committed
    /// the record, and a full event channel must not become back-pressure on
    /// the conversation. A dropped notice is the right trade — the
    /// alternative is a turn that stalls on reporting that history is broken.
    fn report(&self, message: String) {
        if !self.reported.swap(true, Ordering::Relaxed) {
            let _ = self.events.try_send(Event::Notice { message });
        }
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
        session_choices(&self.dir)
            .into_iter()
            .filter(|s| s.id != current.0)
            .collect()
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A submission is on its way to core. Set here as well as by the
    /// `TurnStarted` record, because `/clear` typed straight after a message
    /// is intercepted before core has logged anything.
    pub fn turn_submitted(&self) {
        self.in_turn.store(true, Ordering::Relaxed);
    }

    /// Is a turn still being written? The interceptor's early answer to a
    /// `/clear` or `/resume` while one runs: it says so at once, rather than
    /// reading a transcript that core would then refuse to take. Core's own
    /// refusal is what keeps the writer where it is (`RecordSink::cleared`
    /// and `resumed` are called only when core acts); this flag is only the
    /// earlier word.
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

    /// `/clear` — seal this transcript and begin a new one.
    ///
    /// Sealing is implicit: nothing is written to close the old file. The
    /// developer clears to manage the model's context, not to shred the
    /// record, so the old transcript stays exactly as it is and remains
    /// resumable.
    fn cleared(&self) {
        let id = SessionId::mint();
        let opened = match HistoryStore::create(&self.dir, &id, &header(&self.cwd, &self.model)) {
            Ok(store) => {
                // A new file is a new chance to fail, and to be told about it.
                self.reported.store(false, Ordering::Relaxed);
                Some(store)
            }
            // The old transcript is sealed either way; what is lost is the
            // recording from here on, and the developer is owed that once.
            Err(e) => {
                self.report(format!("history is off from here; the cleared conversation is kept, but no new transcript could be opened: {e}"));
                None
            }
        };
        *self.store.lock().expect("history lock poisoned") = opened;
        *self.current.lock().expect("history lock poisoned") = id;
    }

    /// `/resume` — continue writing into the transcript that was just
    /// loaded, rather than forking a second file for the same conversation.
    fn resumed(&self, session: &SessionId) {
        let reopened = HistoryStore::reopen(&self.dir, session);
        self.reported.store(false, Ordering::Relaxed);
        let store = match reopened {
            Ok(store) => Some(store),
            // The conversation is resumed either way; what cannot happen is
            // writing its continuation into the file it came from — and
            // writing it into the one before would mix two conversations.
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

/// Every session in a history directory, rendered for the picker.
///
/// aldwin-tui takes display halves and nothing else — no paths, no
/// timestamps — so the formatting happens here, the same division the model
/// catalogue already follows.
///
/// Callers inside a running session want [`History::resumable`] instead:
/// this one includes the transcript currently being written.
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

/// `2026-09-20 18:11`, in the developer's own timezone — they are reading
/// back their own afternoon, not a log shipped from elsewhere.
fn format_when(epoch_secs: u64) -> String {
    use chrono::{Local, TimeZone};
    // `try_from`, not `as`: a damaged header past `i64::MAX` would wrap to a
    // plausible-looking date before 1970 rather than to "unknown".
    match i64::try_from(epoch_secs)
        .ok()
        .and_then(|secs| Local.timestamp_opt(secs, 0).single())
    {
        Some(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        // Unrepresentable, which means the header was damaged. The row is
        // still worth showing: its title is what the developer picks by.
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

    /// Step 6: `/clear` seals rather than deletes — the old conversation is
    /// still there to resume, and the new one is a separate file.
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

    /// History must never be able to fail a turn: a store that cannot be
    /// opened costs a message, not a session.
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
        assert!(
            rx.try_recv().is_err(),
            "a store that could not even be reopened is simply off"
        );
    }

    /// A resume whose transcript cannot be reopened stops recording rather
    /// than writing the resumed conversation into the file before it.
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

    /// `/clear` with nowhere to open the next transcript used to switch
    /// history off without a word.
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
