//! Shared test-only fakes and harness helpers, used by this crate's own unit
//! tests across `tools/*.rs` and `dispatcher.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aldwin_core::{DispatchContext, Event, PendingMap, StepId, TurnId};
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::gate::ApprovalGate;

/// An `ApprovalGate` that answers every `request_approval` the same way,
/// without needing a real `DispatchContext` — for tools whose tests don't
/// care about the event side of the round trip (e.g. Read, Run).
pub struct FixedApproval(pub bool);

#[async_trait]
impl ApprovalGate for FixedApproval {
    async fn request_approval(&self, _call_id: String, _diff: String) -> bool {
        self.0
    }
}

pub const ALWAYS_APPROVE: FixedApproval = FixedApproval(true);
pub const ALWAYS_DENY: FixedApproval = FixedApproval(false);

/// Builds a real `DispatchContext` (via core's now-public constructor) plus
/// a held-out clone of the event receiver and the `call_id`-keyed pending
/// map (see `aldwin_core::PendingReply`), so a test can resolve the round
/// trip itself exactly as `Agent`'s command loop does in production.
pub fn dispatch_context() -> (DispatchContext, mpsc::Receiver<Event>, PendingMap) {
    let (tx, rx) = mpsc::channel(16);
    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let ctx = DispatchContext::for_testing(TurnId(1), StepId(1), tx, pending.clone());
    (ctx, rx, pending)
}
