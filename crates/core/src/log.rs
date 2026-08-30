use std::sync::{Arc, RwLock};
use crate::event::LogRecord;

#[derive(Debug, Default, Clone)]
pub struct ConversationLog {
    inner: Arc<RwLock<Vec<LogRecord>>>,
}

impl ConversationLog {
    pub fn new() -> Self { Self::default() }

    pub fn append(&self, record: LogRecord) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TurnId;

    #[test]
    fn clear_empties_a_populated_log() {
        let log = ConversationLog::new();
        log.append(LogRecord::TurnStarted { turn_id: TurnId::next() });
        assert!(!log.is_empty());
        log.clear();
        assert!(log.is_empty());
        assert_eq!(log.snapshot().len(), 0);
    }
}
