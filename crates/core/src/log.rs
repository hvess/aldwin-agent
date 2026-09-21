use std::sync::{Arc, RwLock};
use crate::event::LogRecord;

/// Where a committed record goes *besides* memory.
///
/// Core owns no filesystem dependency and must not grow one, so the
/// transcript writer reaches it as a trait implemented elsewhere
/// (aldwin-config's `HistoryStore`). Core appends records; it never learns
/// where they land, or whether they land at all.
///
/// `append` returns nothing and cannot fail upward by design: history must
/// never be able to fail a turn. A sink that cannot write reports it its own
/// way — `HistoryStore` emits `Event::Notice` once and then stays quiet.
pub trait RecordSink: Send + Sync + std::fmt::Debug {
    fn append(&self, record: &LogRecord);
}

#[derive(Debug, Default, Clone)]
pub struct ConversationLog {
    inner: Arc<RwLock<Vec<LogRecord>>>,
    /// `None` is the ordinary no-history case — every test, and any session
    /// whose store could not be opened.
    sink:  Option<Arc<dyn RecordSink>>,
}

impl ConversationLog {
    pub fn new() -> Self { Self::default() }

    /// The same log, writing through to `sink` as well as to memory.
    pub fn with_sink(sink: Arc<dyn RecordSink>) -> Self {
        Self { inner: Arc::new(RwLock::new(Vec::new())), sink: Some(sink) }
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

    /// An immutable view of every record so far. `.into()` here does clone
    /// each `LogRecord` (`<[T]>::to_owned` under an `Arc<[T]>` conversion) —
    /// it is not free, contrary to what this comment used to claim. O(n)
    /// per call is an accepted cost, same as the streamed-request cost
    /// `wire.rs` documents: this is called once per turn (from
    /// `messages_from_log`, at the top of `run_turn`), not on any hot path,
    /// so it's cheap in the sense of "call frequency," not "cost per call."
    pub fn snapshot(&self) -> Arc<[LogRecord]> {
        let guard = self.inner.read().expect("log lock poisoned");
        guard.as_slice().into()
    }

    pub fn len(&self) -> usize {
        self.inner.read().expect("log lock poisoned").len()
    }

    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// `/clear` — wipes every record so the next turn's
    /// `messages_from_log()` starts from nothing.
    pub fn clear(&self) {
        self.inner.write().expect("log lock poisoned").clear();
    }

    /// `/resume` — the loaded transcript *becomes* the conversation.
    ///
    /// Deliberately not `append`-in-a-loop: these records are already on
    /// disk in the file the session is about to continue writing to, and
    /// replaying them through the sink would write every one of them a
    /// second time. Resume is the one path that fills the log without
    /// filling the transcript.
    pub fn replace(&self, records: Vec<LogRecord>) {
        *self.inner.write().expect("log lock poisoned") = records;
    }

    /// Point the log at a different transcript. `/resume` swaps the writer
    /// to the resumed session's file; `/clear` swaps it to a fresh one.
    pub fn set_sink(&mut self, sink: Option<Arc<dyn RecordSink>>) {
        self.sink = sink;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TurnId;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct Spy(Mutex<Vec<LogRecord>>);

    impl RecordSink for Spy {
        fn append(&self, record: &LogRecord) {
            self.0.lock().unwrap().push(record.clone());
        }
    }

    #[test]
    fn clear_empties_a_populated_log() {
        let log = ConversationLog::new();
        log.append(LogRecord::TurnStarted { turn_id: TurnId::next() });
        assert!(!log.is_empty());
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.snapshot().len(), 0);
    }

    #[test]
    fn a_log_with_no_sink_behaves_exactly_as_before() {
        let log = ConversationLog::new();
        log.append(LogRecord::TurnStarted { turn_id: TurnId::next() });
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn append_fans_out_to_the_sink() {
        let spy = Arc::new(Spy::default());
        let log = ConversationLog::with_sink(spy.clone());
        log.append(LogRecord::UserMessage { turn_id: TurnId::next(), text: "hi".into() });
        assert_eq!(spy.0.lock().unwrap().len(), 1, "the record reaches the sink as well as memory");
        assert_eq!(log.len(), 1);
    }

    /// The resumed records are already in the file this session continues
    /// writing to — replaying them through the sink would duplicate every
    /// one of them on disk.
    #[test]
    fn replace_does_not_write_through_to_the_sink() {
        let spy = Arc::new(Spy::default());
        let log = ConversationLog::with_sink(spy.clone());
        log.replace(vec![LogRecord::UserMessage { turn_id: TurnId(1), text: "from disk".into() }]);
        assert_eq!(log.len(), 1, "the log takes the loaded records");
        assert!(spy.0.lock().unwrap().is_empty(), "and none of them is written back out");
    }
}
