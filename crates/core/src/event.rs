use crate::types::*;
use serde::{Deserialize, Serialize};

// ── LLM-boundary events ─────────────────────────────────────────────────────

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

/// Events the LlmClient yields. Provider-agnostic; no Anthropic wire types cross this boundary.
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
    /// One fragment of thinking text. Carried, not dropped — see
    /// `ContentBlock::Thinking` and ADR 0006 for why.
    ThinkingDelta {
        /// The fragment, to be appended to what came before it.
        text: String,
    },
    /// Closes the block opened by `ThinkingStart`, carrying the whole of it
    /// so the caller can commit one `ContentBlock::Thinking` without having
    /// to re-accumulate the deltas it already saw.
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

// ── Core events (emitted upward) ─────────────────────────────────────────────

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

/// All events the agent emits toward the TUI / future web client.
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
    /// Thinking text as it streams. Nothing draws it yet (open-tasks 24).
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

    /// A turn the developer did not type: the comments they left at a
    /// closing review, started as the next turn's message (ADR 0009 §4).
    /// Sent before that turn's `TurnStarted`, so the TUI can echo what the
    /// model is about to be asked — a typed message it echoes itself.
    FollowUp {
        /// The turn the follow-up is about to start.
        turn_id: TurnId,
        /// The message, as the model will receive it.
        text: String,
    },

    /// The `plan` tool declared or advanced the plan (ADR 0009 §2). The whole
    /// list travels each time, so the TUI holds the latest and nothing else.
    PlanUpdated {
        /// The turn this belongs to.
        turn_id: TurnId,
        /// Every step of the plan, in order.
        steps: Vec<PlanStep>,
    },

    /// The `ask` tool needs an answer. Keyed by the tool call's own id, the
    /// way a review is keyed by its own — a call has at most one round trip
    /// outstanding (see `PendingReply`).
    QuestionAsked {
        /// The `ask` call's id, which `Command::Answer` must echo.
        call_id: String,
        /// What is being asked, and the answers on offer.
        question: Question,
    },

    /// A changeset is staged and about to be observed — by a `run`, or by
    /// the turn ending — so the review opens (ADR 0009 §4). The one gate a
    /// change passes on its way to disk.
    ReviewRequested {
        /// The review's id, which `Command::ReviewDecision` must echo.
        review_id: String,
        /// Every file the turn has staged.
        changeset: Changeset,
    },
    /// How the review ended. Emitted by the dispatcher once the decision has
    /// been acted on — files written, comments handed back, or the
    /// changeset dropped — so the row the conversation keeps describes what
    /// actually happened.
    ReviewClosed {
        /// What was written, commented on, or dropped.
        outcome: ReviewOutcome,
    },

    /// A message from outside the turn/step lifecycle — the session
    /// initialiser (aldwin-cli) rejecting an unknown slash command or
    /// reporting a `/reload-config` result, for example. It exists so a
    /// layer above core (which owns no other vehicle for reaching the TUI's
    /// log) has one. Not turn/step-scoped and never appended to the
    /// conversation log — this is UI-facing only.
    ///
    /// Core emits it in exactly one case, added with ADR 0006: a step that
    /// ends the turn having produced nothing the developer can see. That
    /// used to render as a blank turn and read as a hang; the floor is that
    /// a turn always says *something*, even if only that it said nothing.
    Notice {
        /// The sentence to show.
        message: String,
    },

    /// `Command::ClearHistory` landed and `ConversationLog` was wiped — the
    /// TUI wipes its own rendered log in step.
    HistoryCleared,

    /// `Command::Resume` landed: `ConversationLog` now holds `records` and
    /// nothing else. The exact counterpart of `HistoryCleared` — the TUI
    /// rebuilds its own rendered log from these, the way it wipes its own on
    /// a clear, rather than being told separately by whoever read the file.
    ///
    /// The records travel in the event rather than the TUI reading the
    /// transcript itself: aldwin-tui depends only on core and has no
    /// filesystem access by design, the same reason the model catalogue is
    /// handed to it rather than looked up.
    HistoryLoaded {
        /// The resumed conversation, in order.
        records: Vec<LogRecord>,
    },

    /// `/theme light|dark` — the raw config value; aldwin-tui parses it. Core
    /// never emits this: like `Notice`, it exists because aldwin-cli's
    /// slash-command interceptor has no other vehicle to reach the TUI.
    ThemeChanged {
        /// The raw value, for aldwin-tui to parse.
        theme: String,
    },

    /// `/model` swapped the client the session is running on. Same "a layer
    /// above core has no other vehicle" reasoning as `Notice` and
    /// `ThemeChanged`: core is generic over `C: LlmClient` and has no idea
    /// its client is swappable, so aldwin-cli's interceptor rebuilds the
    /// client behind the trait and announces the result here.
    ///
    /// `model` is the bare model id, the same value the session started
    /// with; `provider` is the catalogue id of the row it belongs to, or
    /// `None` when `provider.yaml` points at an endpoint the catalogue does
    /// not know — the model picker opens on that pair, so both halves have to
    /// travel together. `context_window` is that model's, when the catalogue
    /// knows it, for the context bar.
    ModelChanged {
        /// The catalogue id of the model's provider, if the catalogue knows it.
        provider: Option<String>,
        /// The bare model id.
        model: String,
        /// The model's context window in tokens, if the catalogue knows it.
        context_window: Option<u32>,
    },
}

// ── Commands (accepted downward) ─────────────────────────────────────────────

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
    /// `/clear` — wipes `ConversationLog` so the next turn starts from a
    /// blank slate. Refused with a `Notice` if received mid-turn, same as
    /// `Submit` mid-turn: there's no sound meaning for "forget everything"
    /// while a turn is still in flight using that same history.
    ClearHistory,

    /// `/resume` — the loaded transcript replaces `ConversationLog`, so the
    /// next turn's `messages_from_log()` sees the resumed conversation.
    /// Core acknowledges with `Event::HistoryLoaded`.
    ///
    /// It carries the records as well as the `SessionId` because core owns
    /// no filesystem dependency: aldwin-cli's interceptor reads the file (it
    /// holds the `Config` that knows where history lives) and core is handed
    /// the result. The id goes on to the `RecordSink`, which continues that
    /// session's transcript — at the moment core acts, not before. Same
    /// division as `ClearHistory`, which core acts on without knowing what
    /// `/clear` is.
    ///
    /// Refused with a `Notice` mid-turn, exactly as `ClearHistory` is:
    /// there is no sound meaning for "replace the history" while a turn is
    /// in flight using it.
    Resume {
        /// The conversation being resumed, which the sink continues.
        session: SessionId,
        /// Its records, read from the transcript.
        records: Vec<LogRecord>,
    },
}

// ── Log record ───────────────────────────────────────────────────────────────

/// What gets appended to the conversation log. Mirrors the event set but stripped
/// of streaming-only entries (TextDelta and ThinkingStart/End accumulate into
/// AssistantMessage before being committed).
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
    /// A completed extended-thinking block. Persisted because a resumed
    /// session that dropped it would send the provider an assistant turn
    /// whose tool call has no thinking in front of it, which is rejected —
    /// ADR 0006 §3.
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
