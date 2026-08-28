use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crate::{
    event::Event,
    types::{PromptId, StepId, ToolCall, ToolResult, TurnId},
};

/// Implementors live in amundsen-tools. Approval-gated tools (Edit) block
/// inside their own dispatch future, using `DispatchContext` to ask the
/// developer for a decision; the agent loop just awaits.
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult;

    /// The set of tools available to the model in the current session.
    fn definitions(&self) -> Vec<crate::types::ToolDefinition>;
}

/// Given to a dispatch future so it can request a developer decision without
/// reaching into the agent's internals. Concrete policy — when to gate a
/// tool, how to render a diff, permission-engine rules — lives in
/// amundsen-tools and amundsen-permissions; this only provides the round
/// trip through the agent's existing event/command boundary.
#[derive(Clone)]
pub struct DispatchContext {
    turn_id:   TurnId,
    step_id:   StepId,
    events:    mpsc::Sender<Event>,
    approvals: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
    prompts:   Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
}

impl DispatchContext {
    /// Public so a `ToolDispatcher` implementor (amundsen-tools) can build a
    /// real `DispatchContext` in its own test harness — held-out clones of
    /// `approvals`/`prompts` let a test resolve the round trip itself,
    /// exactly as `Agent`'s command loop does in production.
    pub fn new(
        turn_id:   TurnId,
        step_id:   StepId,
        events:    mpsc::Sender<Event>,
        approvals: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
        prompts:   Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    ) -> Self {
        Self { turn_id, step_id, events, approvals, prompts }
    }

    /// Emit `ToolApprovalRequested` for `call_id` and await the developer's
    /// `ApproveTool`/`DenyTool`. Resolves to `false` (deny) if the agent
    /// shuts down, or the turn is cancelled, before a decision arrives.
    pub async fn request_approval(&self, call_id: String, diff: String) -> bool {
        let (tx, rx) = oneshot::channel();
        self.approvals.lock().expect("approvals lock poisoned").insert(call_id.clone(), tx);
        let _ = self.events.send(Event::ToolApprovalRequested {
            turn_id: self.turn_id, step_id: self.step_id, call_id, diff,
        }).await;
        rx.await.unwrap_or(false)
    }

    /// Emit `PromptRequested` and await the matching `PromptResponse`.
    /// Resolves to `Value::Null` if the agent shuts down before a response
    /// arrives.
    pub async fn request_prompt(&self, payload: serde_json::Value) -> serde_json::Value {
        let id = PromptId::next();
        let (tx, rx) = oneshot::channel();
        self.prompts.lock().expect("prompts lock poisoned").insert(id.0, tx);
        let _ = self.events.send(Event::PromptRequested { id, payload }).await;
        rx.await.unwrap_or(serde_json::Value::Null)
    }
}
