use crate::event::LogRecord;
use crate::types::SessionId;
use std::sync::{Arc, RwLock};

/// Where a committed record goes besides memory; implemented by aldwin-cli's
/// `History` over aldwin-config's `HistoryStore`, so core gains no
/// filesystem dependency.
///
/// Infallible by design: history must never fail a turn. A sink reports its
/// own failures (`History` emits one `Event::Notice`).
///
/// Called synchronously on the agent's task, so a sink must stay a small
/// append to an already-open file; more work needs a writer task.
pub trait RecordSink: Send + Sync + std::fmt::Debug {
    /// A record was committed to the log.
    fn append(&self, record: &LogRecord);

    /// The log was cleared (`/clear`); later records begin a new conversation.
    fn cleared(&self);

    /// The log was replaced by `session`'s records (`/resume`); later records
    /// continue that conversation.
    ///
    /// `cleared` and `resumed` fire when core acts on the command, not when
    /// it is sent: core refuses both mid-turn, and a sink that moved early
    /// would write the rest of that turn into another conversation's file.
    fn resumed(&self, session: &SessionId);
}

/// Every record of the conversation in commit order; each turn's messages are
/// rebuilt from it. Clones share one log.
#[derive(Debug, Default, Clone)]
pub struct ConversationLog {
    inner: Arc<RwLock<Vec<LogRecord>>>,
    /// `None` when there is no history: tests, or a store that could not open.
    sink: Option<Arc<dyn RecordSink>>,
}

impl ConversationLog {
    /// An empty log that writes to memory only.
    pub fn new() -> Self {
        Self::default()
    }

    /// The same log, writing through to `sink` as well as to memory.
    pub fn with_sink(sink: Arc<dyn RecordSink>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Vec::new())),
            sink: Some(sink),
        }
    }

    /// Commits `record`, handing it to the sink first when there is one.
    ///
    /// # Panics
    ///
    /// If the log's lock is poisoned.
    pub fn append(&self, record: LogRecord) {
        // Sink before the lock: a write blocked on disk must not hold readers out.
        if let Some(sink) = &self.sink {
            sink.append(&record);
        }
        self.inner.write().expect("log lock poisoned").push(record);
    }

    /// An immutable copy of every record so far. O(n) clones per call,
    /// acceptable only because the agent calls it once per turn.
    ///
    /// # Panics
    ///
    /// If the log's lock is poisoned.
    pub fn snapshot(&self) -> Arc<[LogRecord]> {
        let guard = self.inner.read().expect("log lock poisoned");
        guard.as_slice().into()
    }

    /// How many records the log holds.
    ///
    /// # Panics
    ///
    /// If the log's lock is poisoned.
    pub fn len(&self) -> usize {
        self.inner.read().expect("log lock poisoned").len()
    }

    /// Whether the log holds no records.
    ///
    /// # Panics
    ///
    /// If the log's lock is poisoned.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `/clear`: wipes every record, so the next turn starts from nothing,
    /// and tells the sink.
    ///
    /// # Panics
    ///
    /// If the log's lock is poisoned.
    pub fn clear(&self) {
        self.inner.write().expect("log lock poisoned").clear();
        if let Some(sink) = &self.sink {
            sink.cleared();
        }
    }

    /// `/resume`: `session`'s loaded transcript becomes the conversation.
    ///
    /// Must not `append` the records: they are already on disk, and the sink
    /// would write each a second time. The sink is only told which session
    /// it now continues.
    ///
    /// # Panics
    ///
    /// If the log's lock is poisoned.
    pub fn replace(&self, session: &SessionId, records: Vec<LogRecord>) {
        *self.inner.write().expect("log lock poisoned") = records;
        if let Some(sink) = &self.sink {
            sink.resumed(session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TurnId;
    use std::sync::Mutex;

    /// Everything the log told its sink, in order.
    #[derive(Debug, Default)]
    struct Spy {
        records: Mutex<Vec<LogRecord>>,
        moves: Mutex<Vec<String>>,
    }

    impl RecordSink for Spy {
        fn append(&self, record: &LogRecord) {
            self.records.lock().unwrap().push(record.clone());
        }
        fn cleared(&self) {
            self.moves.lock().unwrap().push("cleared".into());
        }
        fn resumed(&self, session: &SessionId) {
            self.moves
                .lock()
                .unwrap()
                .push(format!("resumed {session}"));
        }
    }

    fn started() -> LogRecord {
        LogRecord::TurnStarted { turn_id: TurnId(1) }
    }

    #[test]
    fn clear_empties_a_populated_log() {
        let log = ConversationLog::new();
        log.append(started());
        assert!(!log.is_empty());
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.snapshot().len(), 0);
    }

    #[test]
    fn a_log_with_no_sink_behaves_exactly_as_before() {
        let log = ConversationLog::new();
        log.append(started());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn append_fans_out_to_the_sink() {
        let spy = Arc::new(Spy::default());
        let log = ConversationLog::with_sink(spy.clone());
        log.append(LogRecord::UserMessage {
            turn_id: TurnId(1),
            text: "hi".into(),
        });
        assert_eq!(
            spy.records.lock().unwrap().len(),
            1,
            "the record reaches the sink as well as memory"
        );
        assert_eq!(log.len(), 1);
    }

    /// Pins that resumed records are not written to disk a second time.
    #[test]
    fn replace_tells_the_sink_where_it_is_rather_than_writing_through() {
        let spy = Arc::new(Spy::default());
        let log = ConversationLog::with_sink(spy.clone());
        log.replace(
            &SessionId("earlier".into()),
            vec![LogRecord::UserMessage {
                turn_id: TurnId(1),
                text: "from disk".into(),
            }],
        );
        assert_eq!(log.len(), 1, "the log takes the loaded records");
        assert!(
            spy.records.lock().unwrap().is_empty(),
            "and none of them is written back out"
        );
        assert_eq!(*spy.moves.lock().unwrap(), ["resumed earlier"]);
    }

    #[test]
    fn clear_tells_the_sink() {
        let spy = Arc::new(Spy::default());
        let log = ConversationLog::with_sink(spy.clone());
        log.append(started());
        log.clear();
        assert_eq!(*spy.moves.lock().unwrap(), ["cleared"]);
    }
}
