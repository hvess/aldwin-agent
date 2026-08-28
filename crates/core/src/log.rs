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
}
