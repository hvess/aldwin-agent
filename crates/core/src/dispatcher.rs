use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};

use crate::{
    event::Event,
    types::{
        Answer, ChangedFile, Changeset, PlanStep, Question, ReviewDecision, ReviewOutcome, StepId,
        ToolCall, ToolResult, TurnId,
    },
};

/// Runs tool calls; implemented in aldwin-tools. A call that needs the
/// developer (`ask`, a review) awaits the round trip through
/// `DispatchContext` inside its own future.
///
/// `before_step` and `turn_ending` are where the dispatcher opens the review
/// (ADR 0009 §4); core knows nothing about staging.
#[async_trait]
pub trait ToolDispatcher: Send + Sync {
    /// Runs one tool call to completion. A failure is a result with
    /// `is_error` set, which the model reads.
    async fn dispatch(&self, call: ToolCall, ctx: &DispatchContext) -> ToolResult;

    /// The tools available to the model this session.
    fn definitions(&self) -> Vec<crate::types::ToolDefinition>;

    /// Called once per step, before any of `calls` is dispatched. `Some` is
    /// why none of them may run; each is answered with that text as an error
    /// result. `None` proceeds.
    async fn before_step(&self, _calls: &[ToolCall], _ctx: &DispatchContext) -> Option<String> {
        None
    }

    /// Called when the model stopped calling tools and the turn would end.
    /// `Some(text)` is a developer message (review comments) that starts a
    /// new turn at once; `None` lets the turn end.
    async fn turn_ending(&self, _ctx: &DispatchContext) -> Option<String> {
        None
    }
}

/// One outstanding round trip in `PendingMap`: a question keyed by its `ask`
/// call id, a review by an id `DispatchContext::review` mints. One map for
/// both, so cleanup on abort cannot drain one kind and forget the other.
#[derive(Debug)]
pub enum PendingReply {
    /// A question from the `ask` tool, awaiting `Command::Answer`.
    Answer(oneshot::Sender<Answer>),
    /// A review, awaiting `Command::ReviewDecision`.
    Review(oneshot::Sender<ReviewDecision>),
}

/// Round trips waiting on the developer: dispatch futures register them,
/// the agent's command loop resolves them.
pub type PendingMap = Arc<Mutex<HashMap<String, PendingReply>>>;

/// A dispatch future's way to the developer: round trips over the agent's
/// event/command boundary, and one-way announcements. Policy (when a review
/// opens, what a question offers) lives in aldwin-tools.
#[derive(Debug, Clone)]
pub struct DispatchContext {
    turn_id: TurnId,
    step_id: StepId,
    events: mpsc::Sender<Event>,
    pending: PendingMap,
}

impl DispatchContext {
    pub(crate) fn new(
        turn_id: TurnId,
        step_id: StepId,
        events: mpsc::Sender<Event>,
        pending: PendingMap,
    ) -> Self {
        Self {
            turn_id,
            step_id,
            events,
            pending,
        }
    }

    /// Builds a context for a `ToolDispatcher` implementor's tests, which
    /// resolve round trips through their own clone of `pending`.
    ///
    /// Feature-gated instead of making `new` public: outside `Agent`'s run
    /// loop nothing drains the round trip, so it would hang forever.
    #[cfg(any(test, feature = "test-util"))]
    pub fn for_testing(
        turn_id: TurnId,
        step_id: StepId,
        events: mpsc::Sender<Event>,
        pending: PendingMap,
    ) -> Self {
        Self::new(turn_id, step_id, events, pending)
    }

    /// Emits `QuestionAsked` for `call_id` and awaits the developer's
    /// `Answer`; `None` on shutdown or cancellation.
    ///
    /// # Panics
    ///
    /// If the pending-reply lock is poisoned.
    pub async fn ask(&self, call_id: String, question: Question) -> Option<Answer> {
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock poisoned")
            .insert(call_id.clone(), PendingReply::Answer(tx));
        let _ = self
            .events
            .send(Event::QuestionAsked { call_id, question })
            .await;
        rx.await.ok()
    }

    /// Emits `ReviewRequested` and awaits the developer's decision; `None` on
    /// shutdown or cancellation, meaning nothing was written.
    ///
    /// Keyed by step id: a review opens at a step boundary and is answered
    /// before the step goes on, so a step has at most one open.
    ///
    /// # Panics
    ///
    /// If the pending-reply lock is poisoned.
    pub async fn review(&self, changeset: Changeset) -> Option<ReviewDecision> {
        let review_id = format!("review-{}", self.step_id.0);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock poisoned")
            .insert(review_id.clone(), PendingReply::Review(tx));
        let _ = self
            .events
            .send(Event::ReviewRequested {
                review_id,
                changeset,
            })
            .await;
        rx.await.ok()
    }

    /// Announces how a review ended, once the decision has been acted on.
    pub async fn review_closed(&self, outcome: ReviewOutcome) {
        let _ = self.events.send(Event::ReviewClosed { outcome }).await;
    }

    /// Tells the developer something a tool did that they must hear from
    /// Aldwin, not only from the model.
    pub async fn notice(&self, message: String) {
        let _ = self.events.send(Event::Notice { message }).await;
    }

    /// Announces the plan as it now stands.
    pub async fn plan_updated(&self, steps: Vec<PlanStep>) {
        let _ = self
            .events
            .send(Event::PlanUpdated {
                turn_id: self.turn_id,
                steps,
            })
            .await;
    }

    /// Announces the file an edit staged, as it now stands.
    pub async fn staged(&self, file: ChangedFile) {
        let _ = self.events.send(Event::Staged { file }).await;
    }

    /// Drops every review entry; the abort path cannot find one by call id.
    pub(crate) fn clear_reviews(pending: &PendingMap) {
        pending
            .lock()
            .expect("pending lock poisoned")
            .retain(|_, reply| !matches!(reply, PendingReply::Review(_)));
    }
}
