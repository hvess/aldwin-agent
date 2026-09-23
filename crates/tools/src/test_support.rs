//! Shared test-only harness helpers, used by this crate's own unit tests
//! across `tools/*.rs` and `dispatcher.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aldwin_core::{DispatchContext, Event, PendingMap, StepId, TurnId};
use tokio::sync::mpsc;

/// Builds a real `DispatchContext` (via core's test-util constructor) plus
/// a held-out clone of the event receiver and the pending map (see
/// `aldwin_core::PendingReply`), so a test can resolve a round trip itself
/// exactly as `Agent`'s command loop does in production.
pub fn dispatch_context() -> (DispatchContext, mpsc::Receiver<Event>, PendingMap) {
    let (tx, rx) = mpsc::channel(16);
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let ctx = DispatchContext::for_testing(TurnId(1), StepId(1), tx, pending.clone());
    (ctx, rx, pending)
}
