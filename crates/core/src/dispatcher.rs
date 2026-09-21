use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crate::{
    event::Event,
    types::{StepId, ToolCall, ToolResult, TurnId},
};

/// Implementors live in aldwin-tools. Approval-gated tools (Edit) block
/// inside their own dispatch future, using `DispatchContext` to ask the
/// developer for a decision; the agent loop just awaits.
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult;

    /// The set of tools available to the model in the current session.
    fn definitions(&self) -> Vec<crate::types::ToolDefinition>;
}

/// One outstanding `request_approval`/`request_prompt` call, keyed by
/// `call_id` in `PendingMap`. The two round trips resolve to different
/// shapes (a `bool` vs. arbitrary JSON), so this carries whichever one the
/// caller registered. A single call is never mid-approval and mid-prompt at
/// once — Edit-class tools (the only `request_approval` caller) skip the
/// permission check (the only `request_prompt` caller) entirely — so
/// `call_id` alone disambiguates, and one map means cleanup on abort cannot
/// drain one kind and forget the other.
pub enum PendingReply {
    Approval(oneshot::Sender<bool>),
    Prompt(oneshot::Sender<serde_json::Value>),
}

pub type PendingMap = Arc<Mutex<HashMap<String, PendingReply>>>;

/// Given to a dispatch future so it can request a developer decision without
/// reaching into the agent's internals. Concrete policy — when to gate a
/// tool, how to render a diff, permission-engine rules — lives in
/// aldwin-tools and aldwin-permissions; this only provides the round
/// trip through the agent's existing event/command boundary.
#[derive(Clone)]
pub struct DispatchContext {
    turn_id: TurnId,
    step_id: StepId,
    events:  mpsc::Sender<Event>,
    pending: PendingMap,
}

impl DispatchContext {
    pub(crate) fn new(turn_id: TurnId, step_id: StepId, events: mpsc::Sender<Event>, pending: PendingMap) -> Self {
        Self { turn_id, step_id, events, pending }
    }

    /// Lets a `ToolDispatcher` implementor (aldwin-tools) build a real
    /// context in its own test harness, holding a clone of `pending` to
    /// resolve the round trip itself as `Agent`'s command loop would.
    /// Feature-gated rather than making `new` `pub`: a context built outside
    /// `Agent`'s run loop has no `Command` handler draining it, so
    /// `request_approval`/`request_prompt` would hang forever.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_testing(turn_id: TurnId, step_id: StepId, events: mpsc::Sender<Event>, pending: PendingMap) -> Self {
        Self::new(turn_id, step_id, events, pending)
    }

    /// Emit `ToolApprovalRequested` for `call_id` and await the developer's
    /// `ApproveTool`/`DenyTool`. Resolves to `false` (deny) if the agent
    /// shuts down, or the turn is cancelled, before a decision arrives.
    pub async fn request_approval(&self, call_id: String, diff: String) -> bool {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock poisoned").insert(call_id.clone(), PendingReply::Approval(tx));
        let _ = self.events.send(Event::ToolApprovalRequested {
            turn_id: self.turn_id, step_id: self.step_id, call_id, diff,
        }).await;
        rx.await.unwrap_or(false)
    }

    /// Emit `PromptRequested` for `call_id` and await the matching
    /// `PromptResponse`. Resolves to `Value::Null` if the agent shuts down
    /// before a response arrives.
    pub async fn request_prompt(&self, call_id: String, payload: serde_json::Value) -> serde_json::Value {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock poisoned").insert(call_id.clone(), PendingReply::Prompt(tx));
        let _ = self.events.send(Event::PromptRequested { call_id, payload }).await;
        rx.await.unwrap_or(serde_json::Value::Null)
    }
}
