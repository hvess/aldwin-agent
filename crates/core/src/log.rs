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

    /// Cheap snapshot — callers get an immutable view without cloning each record.
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
