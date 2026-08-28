//! Shared test-only fakes and harness helpers, used by this crate's own unit
//! tests across `tools/*.rs` and `dispatcher.rs`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use amundsen_core::{DispatchContext, Event, StepId, TurnId};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};

use crate::gate::ApprovalGate;

/// An `ApprovalGate` that answers every `request_approval` the same way,
/// without needing a real `DispatchContext` — for tools whose tests don't
/// care about the event side of the round trip (e.g. Read, shell).
pub struct FixedApproval(pub bool);

#[async_trait]
impl ApprovalGate for FixedApproval {
    async fn request_approval(&self, _call_id: String, _diff: String) -> bool {
        self.0
    }
}

pub const ALWAYS_APPROVE: FixedApproval = FixedApproval(true);
pub const ALWAYS_DENY: FixedApproval = FixedApproval(false);

pub type Approvals = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;
pub type Prompts = Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>;

/// Builds a real `DispatchContext` (via core's now-public constructor) plus
/// held-out clones of the event receiver and the `approvals`/`prompts` maps,
/// so a test can resolve the round trip itself exactly as `Agent`'s command
/// loop does in production.
pub fn dispatch_context() -> (DispatchContext, mpsc::Receiver<Event>, Approvals, Prompts) {
    let (tx, rx) = mpsc::channel(16);
    let approvals = Arc::new(Mutex::new(HashMap::new()));
    let prompts = Arc::new(Mutex::new(HashMap::new()));
    let ctx = DispatchContext::new(TurnId(1), StepId(1), tx, approvals.clone(), prompts.clone());
    (ctx, rx, approvals, prompts)
}
