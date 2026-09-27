use crate::types::*;
use serde::{Deserialize, Serialize};

// LLM-boundary events.

/// How a step ended, and what it cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepOutcome {
    /// Whether the model finished or is waiting on tool results.
    pub stop_reason: StopReason,
    /// Tokens the step read and wrote.
    pub usage: UsageStats,
    /// How much of the input the provider's prompt cache wrote or served.
    pub cache: CacheStats,
}

/// Events an `LlmClient` yields. No Anthropic wire type crosses this boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmEvent {
    /// One fragment of the assistant's prose.
    TextDelta {
        /// The fragment, to be appended to what came before it.
        text: String,
    },
    /// A thinking block opened.
    ThinkingStart,
    /// One fragment of thinking text; carried, not dropped (ADR 0006).
    ThinkingDelta {
        /// The fragment, to be appended to what came before it.
        text: String,
    },
    /// Closes the block opened by `ThinkingStart`, carrying the whole of it,
    /// so the agent commits it without re-accumulating deltas.
    ThinkingEnd {
        /// The whole thinking text.
        text: String,
        /// The provider's signature over it, sent back verbatim.
        signature: String,
    },
    /// A thinking block the provider encrypted. Opaque, echoed back as-is.
    RedactedThinking {
        /// The encrypted payload.
        data: String,
    },
    /// The model asked for a tool call, complete with its input.
    ToolUseRequested {
        /// The call to dispatch.
        call: ToolCall,
    },
    /// The step finished; nothing follows it on this stream.
    StepEnded {
        /// Why it ended and what it cost.
        outcome: StepOutcome,
    },
    /// An attempt failed before anything was emitted and is being retried.
    RetryAttempt {
        /// What failed, and which attempt this is.
        info: RetryInfo,
    },
}

// Core events, emitted upward.

/// Why a turn ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnEndReason {
    /// The model finished and called no more tools.
    EndTurn,
    /// The developer stopped it.
    Cancelled,
    /// A step failed; the sentence says how.
    Error(String),
}

/// Events toward the TUI, from the agent or aldwin-cli's interceptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A turn began.
    TurnStarted {
        /// The turn's id, minted by the agent.
        turn_id: TurnId,
    },

    /// One fragment of the assistant's prose as it streams.
    TextDelta {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The fragment, to be appended to what came before it.
        text: String,
    },
    /// A thinking block opened.
    ThinkingStart {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
    },
    /// Thinking text as it streams. Nothing draws it yet (open-tasks 3).
    ThinkingDelta {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The fragment, to be appended to what came before it.
        text: String,
    },
    /// The thinking block closed.
    ThinkingEnd {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
    },

    /// The model asked for a tool call.
    ToolUseRequested {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The call, with its input.
        call: ToolCall,
    },
    /// A requested call was handed to the dispatcher and is running.
    ToolDispatched {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The id of the call that started.
        call_id: String,
    },
    /// A call finished, successfully or not.
    ToolCompleted {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// What the call returned; its `call_id` names the call.
        result: ToolResult,
    },

    /// A step finished.
    StepEnded {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// Why it ended and what it cost.
        outcome: StepOutcome,
    },
    /// The provider request failed and is being retried.
    RetryAttempt {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// What failed, and which attempt this is.
        info: RetryInfo,
    },

    /// A turn ended.
    TurnEnded {
        /// The turn that ended.
        turn_id: TurnId,
        /// Why it ended.
        reason: TurnEndReason,
    },

    /// Review comments started as the next turn's message (ADR 0009 §4).
    /// Sent before that turn's `TurnStarted` so the TUI can echo it; a typed
    /// message the TUI echoes itself.
    FollowUp {
        /// The turn the follow-up is about to start.
        turn_id: TurnId,
        /// The message, as the model will receive it.
        text: String,
    },

    /// The `plan` tool declared or advanced the plan (ADR 0009 §2). Carries
    /// the whole list each time; the TUI keeps only the latest.
    PlanUpdated {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// Every step of the plan, in order.
        steps: Vec<PlanStep>,
    },

    /// The `ask` tool needs an answer. Keyed by the call id: a call has at
    /// most one round trip outstanding (see `PendingReply`).
    QuestionAsked {
        /// The `ask` call's id, which `Command::Answer` must echo.
        call_id: String,
        /// What is being asked, and the answers on offer.
        question: Question,
    },

    /// A staged changeset is about to be observed (by a `run`, or the turn
    /// ending), so the review opens (ADR 0009 §4).
    ReviewRequested {
        /// The review's id, which `Command::ReviewDecision` must echo.
        review_id: String,
        /// Every file the turn has staged.
        changeset: Changeset,
    },
    /// How the review ended. Emitted by the dispatcher after the decision is
    /// acted on, so it describes what happened.
    ReviewClosed {
        /// What was written, commented on, or dropped.
        outcome: ReviewOutcome,
    },

    /// A sentence for the developer, outside the turn/step lifecycle and
    /// never logged. aldwin-cli sends it (e.g. an unknown slash command);
    /// core sends it for a turn with nothing visible (ADR 0006), dropped
    /// tool calls, a command refused mid-turn, and a completed resume.
    Notice {
        /// The sentence to show.
        message: String,
    },

    /// `Command::ClearHistory` wiped `ConversationLog`; the TUI wipes its own.
    HistoryCleared,

    /// `Command::Resume` replaced `ConversationLog` with `records`; the TUI
    /// rebuilds its own log from them.
    ///
    /// The records travel in the event because aldwin-tui has no filesystem
    /// access by design.
    HistoryLoaded {
        /// The resumed conversation, in order.
        records: Vec<LogRecord>,
    },

    /// `/theme light|dark`. Emitted only by aldwin-cli's interceptor, never
    /// by core.
    ThemeChanged {
        /// The raw value, for aldwin-tui to parse.
        theme: String,
    },

    /// `/model` swapped the session's client. Emitted only by aldwin-cli's
    /// interceptor, which rebuilds the client behind `LlmClient`; core does
    /// not know it is swappable.
    ///
    /// The model picker opens on `provider` and `model`, so both must travel
    /// together.
    ModelChanged {
        /// The catalogue id of the model's provider; `None` when
        /// `provider.yaml` points at an endpoint the catalogue does not know.
        provider: Option<String>,
        /// The bare model id, as the session started with.
        model: String,
        /// The model's context window in tokens, if the catalogue knows it;
        /// drives the context bar.
        context_window: Option<u32>,
    },
}

// Commands, accepted downward.

/// What the TUI (or aldwin-cli's interceptor) asks of the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// The developer sent a message; it starts a turn. Refused with a
    /// `Notice` mid-turn.
    Submit {
        /// The message as typed.
        text: String,
    },
    /// Stop the running turn.
    Cancel,
    /// The developer's answer to `Event::QuestionAsked`.
    Answer {
        /// The `call_id` the question was asked under.
        call_id: String,
        /// The option they chose, or what they typed.
        answer: Answer,
    },
    /// The developer's decision at `Event::ReviewRequested`.
    ReviewDecision {
        /// The `review_id` the review was requested under.
        review_id: String,
        /// Approve, comment or discard.
        decision: ReviewDecision,
    },
    /// `/clear`: wipes `ConversationLog`. Refused with a `Notice` mid-turn.
    ClearHistory,

    /// `/resume`: the loaded transcript replaces `ConversationLog`; core
    /// answers with `Event::HistoryLoaded`. Refused with a `Notice` mid-turn.
    ///
    /// aldwin-cli reads the records, since core has no filesystem
    /// dependency; `session` goes to the `RecordSink` only when core acts.
    Resume {
        /// The conversation being resumed, which the sink continues.
        session: SessionId,
        /// Its records, read from the transcript.
        records: Vec<LogRecord>,
    },
}

// Log record.

/// An entry in the conversation log. No streaming-only entries: deltas are
/// committed accumulated, as `AssistantMessage` and `Thinking`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LogRecord {
    /// A turn began.
    TurnStarted {
        /// The turn's id.
        turn_id: TurnId,
    },
    /// The message that started a turn — typed, or a review's follow-up.
    UserMessage {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The message.
        text: String,
    },
    /// A step's prose, accumulated from its deltas.
    AssistantMessage {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The whole text.
        text: String,
    },
    /// A completed extended-thinking block. Must be persisted: a resumed
    /// tool call without its thinking is rejected by the provider (ADR 0006
    /// §3).
    Thinking {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The whole thinking text.
        text: String,
        /// The provider's signature over it, sent back verbatim.
        signature: String,
    },
    /// The encrypted counterpart, kept for the same reason.
    RedactedThinking {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The encrypted payload.
        data: String,
    },
    /// A tool call the model made.
    ToolUse {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The call, with its input.
        call: ToolCall,
    },
    /// What a tool call returned.
    ToolResult {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// The result; its `call_id` names the call.
        result: crate::types::ToolResult,
    },
    /// A step ended.
    StepBoundary {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// The step within that turn.
        step_id: StepId,
        /// Why it ended and what it cost.
        outcome: StepOutcome,
    },
    /// A turn ended.
    TurnEnded {
        /// The turn that ended.
        turn_id: TurnId,
        /// Why it ended.
        reason: TurnEndReason,
    },
}

impl LogRecord {
    /// The turn this record belongs to, and its step when it has one.
    pub(crate) fn ids(&self) -> (TurnId, Option<StepId>) {
        match self {
            Self::TurnStarted { turn_id }
            | Self::UserMessage { turn_id, .. }
            | Self::TurnEnded { turn_id, .. } => (*turn_id, None),
            Self::AssistantMessage {
                turn_id, step_id, ..
            }
            | Self::Thinking {
                turn_id, step_id, ..
            }
            | Self::RedactedThinking {
                turn_id, step_id, ..
            }
            | Self::ToolUse {
                turn_id, step_id, ..
            }
            | Self::ToolResult {
                turn_id, step_id, ..
            }
            | Self::StepBoundary {
                turn_id, step_id, ..
            } => (*turn_id, Some(*step_id)),
        }
    }
}
