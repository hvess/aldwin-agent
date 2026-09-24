use crate::event::LogRecord;
use crate::types::SessionId;
use std::sync::{Arc, RwLock};

/// Where a committed record goes *besides* memory.
///
/// Core owns no filesystem dependency and must not grow one, so the
/// transcript writer reaches it as a trait implemented elsewhere
/// (aldwin-cli's `History`, over aldwin-config's `HistoryStore`). Core
/// appends records; it never learns where they land, or whether they land
/// at all.
///
/// Nothing here returns anything and nothing can fail upward, by design:
/// history must never be able to fail a turn. A sink that cannot write
/// reports it its own way — `History` emits `Event::Notice` once and then
/// stays quiet.
///
/// Called synchronously, on the agent's task. A sink's work per record is
/// one small append to a file it already holds open, which is cheaper than
/// the hand-off to a writer task would be; a sink that did more would
/// need that task.
pub trait RecordSink: Send + Sync + std::fmt::Debug {
    fn append(&self, record: &LogRecord);

    /// The log was cleared (`/clear`): the records after this begin a new
    /// conversation, and belong somewhere new.
    fn cleared(&self);

    /// The log was replaced by `session`'s records (`/resume`): the records
    /// after this continue that conversation, and belong where it is kept.
    ///
    /// Both of these are called when core *acts* on the command, not when
    /// it is sent: core refuses either one while a turn runs, and a sink
    /// that moved on the way past would send the rest of that turn into
    /// another conversation's file.
    fn resumed(&self, session: &SessionId);
}

#[derive(Debug, Default, Clone)]
pub struct ConversationLog {
    inner: Arc<RwLock<Vec<LogRecord>>>,
    /// `None` is the ordinary no-history case — every test, and any session
    /// whose store could not be opened.
    sink: Option<Arc<dyn RecordSink>>,
}

impl ConversationLog {
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

    pub fn append(&self, record: LogRecord) {
        // The sink sees the record before the lock is taken, not inside it:
        // a write that blocks on disk must not hold every other reader of
        // the log out for its duration.
        if let Some(sink) = &self.sink {
            sink.append(&record);
        }
        self.inner.write().expect("log lock poisoned").push(record);
    }

    /// An immutable view of every record so far. Clones each `LogRecord` —
    /// O(n) per call, accepted because it is called once per turn (from
    /// `messages_from_log`) and not on any hot path.
    pub fn snapshot(&self) -> Arc<[LogRecord]> {
        let guard = self.inner.read().expect("log lock poisoned");
        guard.as_slice().into()
    }

    pub fn len(&self) -> usize {
        self.inner.read().expect("log lock poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `/clear` — wipes every record so the next turn's
    /// `messages_from_log()` starts from nothing, and tells the sink.
    pub fn clear(&self) {
        self.inner.write().expect("log lock poisoned").clear();
        if let Some(sink) = &self.sink {
            sink.cleared();
        }
    }

    /// `/resume` — `session`'s loaded transcript *becomes* the conversation.
    ///
    /// Deliberately not `append`-in-a-loop: these records are already on
    /// disk in the file the session is about to continue writing to, and
    /// replaying them through the sink would write every one of them a
    /// second time. Resume is the one path that fills the log without
    /// filling the transcript; the sink is told which conversation it now
    /// continues instead.
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

    /// The resumed records are already in the file this session continues
    /// writing to — replaying them through the sink would duplicate every
    /// one of them on disk. The sink is told which conversation it now
    /// continues instead.
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
