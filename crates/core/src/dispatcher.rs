use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crate::{
    event::Event,
    types::{Answer, Changeset, PlanStep, Question, ReviewDecision, ReviewOutcome, StepId, ToolCall, ToolResult, TurnId},
};

/// Implementors live in aldwin-tools. A tool that needs the developer — the
/// `ask` tool, or the review a staged changeset opens — blocks inside its own
/// future, using `DispatchContext` for the round trip; the agent loop just
/// awaits.
///
/// Two hooks bracket a step (ADR 0009 §4). They exist so the dispatcher can
/// open the review at the moments a staged change would otherwise be
/// observed without one: before a step whose calls would see the disk, and
/// when the turn is about to end. Core knows nothing about staging; it only
/// provides the two moments.
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult;

    /// The set of tools available to the model in the current session.
    fn definitions(&self) -> Vec<crate::types::ToolDefinition>;

    /// Called once per step, before any of `calls` is dispatched. `Some` is
    /// a reason the step's calls must **not** run — every one of them is then
    /// answered with that text as an error result, and the model decides
    /// what to do next. `None` proceeds.
    async fn before_step(&self, _calls: &[ToolCall], _ctx: &DispatchContext) -> Option<String> {
        None
    }

    /// Called when a turn is about to end because the model stopped calling
    /// tools. `Some(text)` is a message from the developer that starts a new
    /// turn immediately — the comments left at a review — and `None` lets
    /// the turn end.
    async fn turn_ending(&self, _ctx: &DispatchContext) -> Option<String> {
        None
    }
}

/// One outstanding round trip, keyed by its id in `PendingMap`. A question
/// is keyed by the `ask` call's own id; a review by an id the context mints,
/// since a review is not a call. The two resolve to different shapes, so
/// this carries whichever one the caller registered; one map means cleanup
/// on abort cannot drain one kind and forget the other.
pub enum PendingReply {
    Answer(oneshot::Sender<Answer>),
    Review(oneshot::Sender<ReviewDecision>),
}

pub type PendingMap = Arc<Mutex<HashMap<String, PendingReply>>>;

static NEXT_REVIEW: AtomicU64 = AtomicU64::new(1);

/// Given to a dispatch future so it can reach the developer without reaching
/// into the agent's internals. Concrete policy — when a review opens, what a
/// question offers — lives in aldwin-tools; this only provides the round
/// trips through the agent's existing event/command boundary, and the two
/// one-way announcements the TUI draws from.
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
    /// `Agent`'s run loop has no `Command` handler draining it, so a round
    /// trip would hang forever.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_testing(turn_id: TurnId, step_id: StepId, events: mpsc::Sender<Event>, pending: PendingMap) -> Self {
        Self::new(turn_id, step_id, events, pending)
    }

    /// Emit `QuestionAsked` for `call_id` and await the developer's `Answer`.
    /// Resolves to `None` if the agent shuts down, or the turn is cancelled,
    /// before an answer arrives.
    pub async fn ask(&self, call_id: String, question: Question) -> Option<Answer> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock poisoned").insert(call_id.clone(), PendingReply::Answer(tx));
        let _ = self.events.send(Event::QuestionAsked { call_id, question }).await;
        rx.await.ok()
    }

    /// Emit `ReviewRequested` and await the developer's decision. Resolves to
    /// `None` on shutdown or cancellation — the caller treats that as
    /// "nothing was written", which is also what it means.
    pub async fn review(&self, changeset: Changeset) -> Option<ReviewDecision> {
        let review_id = format!("review-{}", NEXT_REVIEW.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock poisoned").insert(review_id.clone(), PendingReply::Review(tx));
        let _ = self.events.send(Event::ReviewRequested { review_id, changeset }).await;
        rx.await.ok()
    }

    /// Announce how a review ended, once the decision has been acted on.
    pub async fn review_closed(&self, outcome: ReviewOutcome) {
        let _ = self.events.send(Event::ReviewClosed { outcome }).await;
    }

    /// The step this context was built for — for a dispatcher that wants
    /// to key something by step.
    pub fn step_id(&self) -> StepId {
        self.step_id
    }

    /// Announce the plan as it now stands.
    pub async fn plan_updated(&self, steps: Vec<PlanStep>) {
        let _ = self.events.send(Event::PlanUpdated { turn_id: self.turn_id, steps }).await;
    }

    /// Drops every review entry — a review is not a call, so the abort path
    /// cannot find it by call id.
    pub(crate) fn clear_reviews(pending: &PendingMap) {
        pending.lock().expect("pending lock poisoned").retain(|_, reply| !matches!(reply, PendingReply::Review(_)));
    }
}
